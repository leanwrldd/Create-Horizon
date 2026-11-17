//! The Hose Pulley's rules: how far its hose is let down and when it stops, how it drains the body
//! of fluid it reaches or fills a space, when a body counts as infinite, and what its small tank
//! does with fluid pushed in or pulled out. Ported from Create (MIT, the Creators of Create),
//! `mc1.21.1/dev` at 4148fd8: `HosePulleyBlockEntity` (`onSpeedChanged`, `tick`, `lazyTick`,
//! `getMovementSpeed`, `getInterpolatedOffset`), `HosePulleyFluidHandler`, `HosePulleyBlock`,
//! `FluidManipulationBehaviour` (`search`, `comparePositions`, the validation timers),
//! `FluidDrainingBehaviour` (`pullNext`, `checkValid`, `tick`, the search and validation),
//! `FluidFillingBehaviour` (`tryDeposit`, `getAtPos`, `canBeReplacedByFluid`), `CFluids` (the
//! range, the threshold, `fillInfinite`, `fluidFillPlaceFluidSourceBlocks`), the
//! `create:bottomless/allow` tag (water and lava) and `CStress` (`setImpact(4.0)`).
//!
//! No game types: the glue is `native/api/src/machines/hose_pulley.rs`. It tells these rules about
//! the world through [`Level`] and [`Edit`] and applies what they ask for.
//!
//! Known differences from Create:
//! - The searches' state isn't saved (`LastPos`, `AffectedAreaFrom`, `AffectedAreaTo`): after a
//!   reload the hose searches again from where it hangs, as if its root had changed.
//! - No fluid ticks are scheduled over the affected area when a search is reset
//!   (`scheduleUpdatesInAffectedArea`) and none are cleared after a fill, as Horizon has no call
//!   for it: drained and filled water reacts to its neighbours when the game next updates them.
//! - A block above a drained cell that can't survive without the fluid isn't broken
//!   (`canSurvive`), and the advancements aren't awarded.
//! - Waterlogged blocks are drained and filled with water only, as Create's, but which blocks have a
//!   fluid and no collision (kelp, seagrass, a bubble column) is by name.
//! - A fill into a block that isn't air or fluid breaks it with drops where Create calls
//!   `Block.dropResources`; what the game calls `blocksMotion` is read as "has a collision box".

use std::collections::{BTreeSet, HashSet, VecDeque};

use crate::rules::piston::convert_to_linear;
use crate::{Dir, Pos};

pub const HOSE_PULLEY: &str = "create:hose_pulley";
/// `CStress.setImpact(4.0)` (`AllBlocks.HOSE_PULLEY`).
pub const IMPACT: f32 = 4.0;
/// `new SmartFluidTank(1500, ..)`.
pub const TANK_CAPACITY: i32 = 1500;
/// A bucket: what one `pullNext` takes and one deposit places.
pub const BUCKET: i32 = 1000;
/// The partial models' smallest drawn offset (`getInterpolatedOffset`).
pub const MIN_DRAWN_OFFSET: f32 = 3.0 / 16.0;

/// The server's config for the hose (`CFluids`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// `hosePulleyRange`: how far from the hose's end a fluid block may be.
    pub range: i32,
    /// `hosePulleyBlockThreshold`: how many blocks of fluid make a body infinite; -1 for never.
    pub max_blocks: i32,
    /// `fillInfinite`: whether an infinite body is filled on.
    pub fill_infinite: bool,
    /// `fluidFillPlaceFluidSourceBlocks`.
    pub place_sources: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { range: 128, max_blocks: 10_000, fill_infinite: false, place_sources: true }
    }
}

/// `FluidManipulationBehaviour.searchedPerTick`.
pub const SEARCHED_PER_TICK: usize = 1024;
/// `FluidManipulationBehaviour.validationTimerMin`.
pub const VALIDATION_TIMER_MIN: i32 = 160;

/// `BottomlessFluidMode.ALLOW_BY_TAG` with `create:bottomless/allow` (water and lava).
pub fn bottomless(fluid: &str) -> bool {
    matches!(fluid, "minecraft:water" | "minecraft:lava")
}

/// `FluidHelper.hasBlockState`: the fluids that have a block to place (not tea or potions).
pub fn has_block_state(fluid: &str) -> bool {
    matches!(fluid, "minecraft:water" | "minecraft:lava" | "create:honey" | "create:chocolate")
}

impl Config {
    /// `FluidManipulationBehaviour.validationTimer`.
    pub fn validation_timer(&self) -> i32 {
        if self.max_blocks < 0 {
            VALIDATION_TIMER_MIN
        } else {
            VALIDATION_TIMER_MIN.max(self.max_blocks / SEARCHED_PER_TICK as i32 + 1)
        }
    }

    /// `FluidManipulationBehaviour.canDrainInfinitely`.
    pub fn can_drain_infinitely(&self, fluid: Option<&str>) -> bool {
        fluid.is_some_and(bottomless) && self.max_blocks != -1
    }
}

// --- the hose's own motion ----------------------------------------------------------------------

/// `HosePulleyBlockEntity.getMovementSpeed`: blocks a tick the hose is let down at `rpm` (no clamp).
pub fn movement_speed(rpm: f32) -> f32 {
    convert_to_linear(rpm)
}

/// What the hose is doing at a snapshot: where it hung at `time` and whether it was moving.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub offset: f32,
    pub time: i64,
    pub moving: bool,
}

impl Motion {
    pub fn new() -> Self {
        Motion { offset: 0.0, time: 0, moving: true }
    }

    /// The offset `now`, as a client follows it between snapshots: a hose that isn't moving
    /// hangs where it was left.
    pub fn offset_at(&self, rpm: f32, now: f64) -> f32 {
        if !self.moving {
            return self.offset;
        }
        (self.offset + movement_speed(rpm) * (now - self.time as f64) as f32).max(0.0)
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self::new()
    }
}

/// `(int) Math.ceil(offset)`: how many blocks below the pulley the hose's end block is.
pub fn end_depth(offset: f32) -> i32 {
    offset.ceil() as i32
}

/// The cell below the pulley at `pos` that is `depth` blocks down.
pub fn below(pos: Pos, depth: i32) -> Pos {
    [pos[0], pos[1] - depth, pos[2]]
}

/// `HosePulleyBlockEntity.tick` (both sides run it every tick): the offset after a tick of
/// `speed` blocks, and whether the hose is still moving. `replaceable(depth)` says whether the
/// block `depth` below the pulley `canBeReplaced`.
pub fn tick(
    offset: f32,
    moving: bool,
    speed: f32,
    rpm_is_zero: bool,
    replaceable: &dyn Fn(i32) -> bool,
) -> (f32, bool) {
    let mut moving = moving;
    let mut next = offset + speed;
    if next < 0.0 {
        next = 0.0;
        moving = false;
    }
    if !replaceable(end_depth(next)) {
        next = next.trunc();
        moving = false;
    }
    if rpm_is_zero {
        moving = false;
    }
    (next, moving)
}

/// `HosePulleyBlockEntity.onSpeedChanged`: the offset and `isMoving` after the speed changed to
/// `rpm`. A hose that moves again starts its searches afresh (`reset`: the second result).
pub fn speed_changed(offset: f32, rpm: f32, replaceable: &dyn Fn(i32) -> bool) -> (f32, bool, bool) {
    let mut offset = offset;
    let mut moving = true;
    if rpm == 0.0 {
        // `offset.setValue(Math.round(offset.getValue()))`.
        offset = (offset + 0.5).floor();
        moving = false;
    }
    if moving {
        let next = offset + movement_speed(rpm);
        if next < 0.0 {
            moving = false;
        }
        if !replaceable(end_depth(next)) {
            moving = false;
        }
    }
    (offset, moving, moving)
}

/// `HosePulleyBlockEntity.lazyTick` for a hose that isn't moving: whether it starts again. Only a
/// positive (lowering) speed does, and only while the block below its next position is free.
pub fn restarts(offset: f32, rpm: f32, replaceable: &dyn Fn(i32) -> bool) -> bool {
    let speed = movement_speed(rpm);
    let ceil = end_depth(offset + speed);
    speed > 0.0 && replaceable(ceil)
}

/// `getInterpolatedOffset`: what is drawn, never above a sixteenth-and-a-bit.
pub fn drawn_offset(offset: f32) -> f32 {
    offset.max(MIN_DRAWN_OFFSET)
}

// --- the cells the searches read ------------------------------------------------------------------

/// What a cell holds, as the behaviours read it.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    /// A block with a `waterlogged` property, and whether it is.
    Waterlogged(bool),
    /// A liquid block (`LiquidBlock`): the fluid it holds (still) and its `level` (0 a source).
    Liquid { fluid: String, level: i32 },
    /// A block that holds a source fluid with no collision of its own (kelp, seagrass, a bubble
    /// column: `getFluidState` not empty and an empty collision shape).
    Wet { fluid: String },
    /// Anything else; whether the fluid may replace it (`canBeReplacedByFluid`).
    Other { replaceable_by_fluid: bool },
}

impl Cell {
    /// `Level.getFluidState`: the still fluid in the cell and whether it is a source.
    pub fn fluid_state(&self) -> Option<(&str, bool)> {
        match self {
            Cell::Waterlogged(true) => Some(("minecraft:water", true)),
            Cell::Liquid { fluid, level } => Some((fluid, *level == 0)),
            Cell::Wet { fluid } => Some((fluid, true)),
            _ => None,
        }
    }
}

/// How the glue answers the rules' questions about the world.
pub trait Level {
    fn loaded(&self, pos: Pos) -> bool;
    fn cell(&self, pos: Pos) -> Cell;
    /// `dimensionType().ultraWarm()`.
    fn ultra_warm(&self) -> bool {
        false
    }
}

/// What a drain or a fill changes in a cell.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Air where a fluid was.
    Air,
    /// `waterlogged=false`.
    Unwaterlog,
    /// `waterlogged=true`.
    Waterlog,
    /// A source block of the fluid, after breaking what was there (`replaceBlock`).
    Source(String),
}

/// A [`Level`] the behaviours may change.
pub trait Edit: Level {
    fn set(&mut self, pos: Pos, change: Change);
    /// `playEffect`: the sound at `pos` of `fluid` being taken out of the world (`fill_sound`: a
    /// bucket filling) or put into it (a bucket emptying).
    fn effect(&mut self, pos: Pos, fluid: &str, fill_sound: bool);
    /// The sound of water meeting the Nether's heat.
    fn evaporates(&mut self, pos: Pos);
}

pub const DIRECTIONS: [Dir; 6] = [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East];

pub fn relative(pos: Pos, side: Dir) -> Pos {
    let n = side.normal();
    [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
}

fn dist_sq(a: Pos, b: Pos) -> i64 {
    (0..3).map(|i| i64::from(a[i] - b[i]).pow(2)).sum()
}

/// A chunk the search met wasn't loaded (`ChunkNotLoadedException`).
#[derive(Debug, PartialEq)]
pub struct NotLoaded;

/// `FluidManipulationBehaviour.search`: a breadth-first walk over the cells of one fluid. Takes up
/// to [`SEARCHED_PER_TICK`] cells off `frontier` and calls `add` for each fluid cell it keeps (with
/// the frontier, for the validation of the filler that adds to it). Returns the fluid it found.
#[allow(clippy::too_many_arguments)]
pub fn search(
    level: &dyn Level,
    cfg: &Config,
    root: Pos,
    fluid: Option<&str>,
    frontier: &mut VecDeque<(Pos, i32)>,
    visited: &mut HashSet<Pos>,
    add: &mut dyn FnMut(Pos, i32, &mut VecDeque<(Pos, i32)>),
    search_downward: bool,
) -> Result<Option<String>, NotLoaded> {
    let mut fluid: Option<String> = fluid.map(str::to_owned);
    let range_sq = i64::from(cfg.range) * i64::from(cfg.range);
    let mut i = 0;
    while i < SEARCHED_PER_TICK
        && !frontier.is_empty()
        && (visited.len() as i64 <= i64::from(cfg.max_blocks) || !cfg.can_drain_infinitely(fluid.as_deref()))
    {
        i += 1;
        let Some((current, distance)) = frontier.pop_front() else { break };
        if visited.contains(&current) {
            continue;
        }
        visited.insert(current);
        if !level.loaded(current) {
            return Err(NotLoaded);
        }
        let cell = level.cell(current);
        let Some((current_fluid, _)) = cell.fluid_state() else { continue };
        let current_fluid = current_fluid.to_owned();
        if fluid.is_none() {
            fluid = Some(current_fluid.clone());
        }
        if fluid.as_deref() != Some(current_fluid.as_str()) {
            continue;
        }
        add(current, distance, frontier);
        for side in DIRECTIONS {
            if !search_downward && side == Dir::Down {
                continue;
            }
            let offset = relative(current, side);
            if !level.loaded(offset) {
                return Err(NotLoaded);
            }
            if visited.contains(&offset) {
                continue;
            }
            if dist_sq(offset, root) > range_sq {
                continue;
            }
            let next = level.cell(offset);
            let Some((_, source)) = next.fluid_state() else { continue };
            // A flowing fluid above is only followed straight up the hose's own column
            // (`nextFluid == convertToFlowing(nextFluid) && side == UP && !onSameAxis`).
            if !source && side == Dir::Up && !(offset[0] == root[0] && offset[2] == root[2]) {
                continue;
            }
            frontier.push_back((offset, distance + 1));
        }
    }
    Ok(fluid)
}

/// `FluidManipulationBehaviour.comparePositions` as a key whose smallest comes first: the
/// highest cells first, then the farthest from the hose by search distance, then by distance in
/// space. A filler reads it the other way round (`-comparePositions`).
#[derive(Clone, Debug, Default)]
pub struct Ranked {
    set: BTreeSet<(i32, i32, i64, Pos)>,
    reversed: bool,
}

impl Ranked {
    pub fn drain_order() -> Self {
        Ranked { set: BTreeSet::new(), reversed: false }
    }

    pub fn fill_order() -> Self {
        Ranked { set: BTreeSet::new(), reversed: true }
    }

    fn key(&self, pos: Pos, distance: i32, root: Pos) -> (i32, i32, i64, Pos) {
        let d2 = dist_sq(pos, root);
        if self.reversed {
            (pos[1], distance, d2, pos)
        } else {
            (-pos[1], -distance, -d2, pos)
        }
    }

    pub fn enqueue(&mut self, pos: Pos, distance: i32, root: Pos) {
        let key = self.key(pos, distance, root);
        self.set.insert(key);
    }

    /// The first entry: its cell and search distance.
    pub fn first(&self) -> Option<(Pos, i32)> {
        let key = self.set.iter().next()?;
        Some((key.3, if self.reversed { key.1 } else { -key.1 }))
    }

    pub fn dequeue(&mut self) {
        if let Some(first) = self.set.iter().next().copied() {
            self.set.remove(&first);
        }
    }

    pub fn clear(&mut self) {
        self.set.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// The entries in order: cell and search distance.
    pub fn iter(&self) -> impl Iterator<Item = (Pos, i32)> + '_ {
        let reversed = self.reversed;
        self.set.iter().map(move |key| (key.3, if reversed { key.1 } else { -key.1 }))
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }
}

/// The state both behaviours share (`FluidManipulationBehaviour`).
#[derive(Clone, Debug)]
struct Base {
    root: Option<Pos>,
    infinite: bool,
    counterpart_acted: bool,
    frontier: VecDeque<(Pos, i32)>,
    visited: HashSet<Pos>,
    revalidate_in: i32,
}

impl Base {
    fn new(cfg: &Config) -> Self {
        Base {
            root: None,
            infinite: false,
            counterpart_acted: false,
            frontier: VecDeque::new(),
            visited: HashSet::new(),
            revalidate_in: cfg.validation_timer(),
        }
    }

    /// `FluidManipulationBehaviour.reset`.
    fn reset(&mut self, cfg: &Config) {
        self.revalidate_in = cfg.validation_timer();
        self.frontier.clear();
        self.visited.clear();
        self.infinite = false;
    }
}

// --- draining -------------------------------------------------------------------------------------

/// `FluidBlockType` of `FluidDrainingBehaviour.canPullFluidsFrom`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pull {
    None,
    Source,
    Flowing,
}

fn can_pull_from(cell: &Cell) -> Pull {
    match cell {
        Cell::Waterlogged(true) => Pull::Source,
        Cell::Liquid { level, .. } => {
            if *level == 0 { Pull::Source } else { Pull::Flowing }
        }
        Cell::Wet { .. } => Pull::Source,
        _ => Pull::None,
    }
}

/// `FluidDrainingBehaviour`: finds the body of fluid at the hose's end and takes a bucket of it at
/// a time, from the top down, the farthest first. A body of `max_blocks` or more of a fluid that
/// may be bottomless is infinite and never runs out.
#[derive(Clone, Debug)]
pub struct Drainer {
    base: Base,
    own: Pos,
    fluid: Option<String>,
    validation_set: HashSet<Pos>,
    queue: Ranked,
    is_valid: bool,
    validation_frontier: VecDeque<(Pos, i32)>,
    validation_visited: HashSet<Pos>,
    new_validation_set: HashSet<Pos>,
    /// Set when something the goggles show changed (`blockEntity.sendData`).
    pub dirty: bool,
}

impl Drainer {
    /// A drainer of the pulley at `own`.
    pub fn new(own: Pos, cfg: &Config) -> Self {
        Drainer {
            base: Base::new(cfg),
            own,
            fluid: None,
            validation_set: HashSet::new(),
            queue: Ranked::drain_order(),
            is_valid: false,
            validation_frontier: VecDeque::new(),
            validation_visited: HashSet::new(),
            new_validation_set: HashSet::new(),
            dirty: false,
        }
    }

    pub fn infinite(&self) -> bool {
        self.base.infinite
    }

    pub fn counterpart_acted(&mut self) {
        self.base.counterpart_acted = true;
    }

    /// `isSearching`.
    pub fn searching(&self) -> bool {
        !self.base.frontier.is_empty()
    }

    /// `reset` (the drainer's own: everything is forgotten).
    pub fn reset(&mut self, cfg: &Config) {
        self.base.reset(cfg);
        self.fluid = None;
        self.base.root = None;
        self.queue.clear();
        self.validation_set.clear();
        self.new_validation_set.clear();
        self.validation_frontier.clear();
        self.validation_visited.clear();
        self.dirty = true;
    }

    fn soft_reset(&mut self, cfg: &Config, root: Pos) {
        self.queue.clear();
        self.validation_set.clear();
        self.new_validation_set.clear();
        self.validation_frontier.clear();
        self.validation_visited.clear();
        self.base.visited.clear();
        self.base.infinite = false;
        self.base.revalidate_in = cfg.validation_timer();
        self.base.frontier.push_back((root, 0));
        self.dirty = true;
    }

    /// `rebuildContext`.
    pub fn rebuild_context(&mut self, cfg: &Config, root: Pos) {
        self.reset(cfg);
        self.base.root = Some(root);
        if self.is_valid {
            self.base.frontier.push_back((root, 0));
        }
    }

    fn revalidate(&mut self, cfg: &Config, root: Pos) {
        self.validation_frontier.clear();
        self.validation_visited.clear();
        self.new_validation_set.clear();
        self.validation_frontier.push_back((root, 0));
        self.base.revalidate_in = cfg.validation_timer();
    }

    /// `checkValid`: whether the cell at `root`, or the fluid above it, is a source (or has one
    /// beside a flowing cell), reading up the hose's column.
    pub fn check_valid(&self, level: &dyn Level, root: Pos) -> bool {
        let mut current = root;
        let mut timeout = 1000;
        while timeout > 0 && root != self.own {
            timeout -= 1;
            match can_pull_from(&level.cell(current)) {
                Pull::Flowing => {
                    for side in DIRECTIONS {
                        if can_pull_from(&level.cell(relative(current, side))) == Pull::Source {
                            return true;
                        }
                    }
                    current = relative(current, Dir::Up);
                }
                Pull::Source => return true,
                Pull::None => break,
            }
        }
        false
    }

    /// `FluidDrainingBehaviour.tick`.
    pub fn tick(&mut self, level: &dyn Level, cfg: &Config) {
        if let Some(root) = self.base.root {
            self.is_valid = self.check_valid(level, root);
        }
        if !self.base.frontier.is_empty() {
            self.continue_search(level, cfg);
            return;
        }
        if !self.validation_frontier.is_empty() {
            self.continue_validation(level, cfg);
            return;
        }
        if self.base.revalidate_in > 0 {
            self.base.revalidate_in -= 1;
        }
    }

    fn continue_search(&mut self, level: &dyn Level, cfg: &Config) {
        let root = self.base.root.unwrap_or(self.own);
        let mut found: Vec<(Pos, i32)> = Vec::new();
        let result = search(
            level,
            cfg,
            root,
            self.fluid.as_deref(),
            &mut self.base.frontier,
            &mut self.base.visited,
            &mut |pos, distance, _| found.push((pos, distance)),
            false,
        );
        for (pos, distance) in found {
            self.queue.enqueue(pos, distance, root);
            self.validation_set.insert(pos);
        }
        match result {
            Ok(fluid) => self.fluid = fluid,
            Err(NotLoaded) => {
                self.dirty = true;
                self.base.frontier.clear();
                self.base.visited.clear();
            }
        }
        if self.base.visited.len() as i64 >= i64::from(cfg.max_blocks)
            && cfg.can_drain_infinitely(self.fluid.as_deref())
            && !self.queue.is_empty()
        {
            self.base.infinite = true;
            let first = self.queue.first().map_or(root, |(pos, _)| pos);
            self.base.frontier.clear();
            self.base.visited.clear();
            self.queue.clear();
            self.queue.enqueue(first, 0, root);
            self.dirty = true;
            return;
        }
        if !self.base.frontier.is_empty() {
            return;
        }
        self.dirty = true;
        self.base.visited.clear();
    }

    fn continue_validation(&mut self, level: &dyn Level, cfg: &Config) {
        let root = self.base.root.unwrap_or(self.own);
        let mut found: Vec<Pos> = Vec::new();
        let result = search(
            level,
            cfg,
            root,
            self.fluid.as_deref(),
            &mut self.validation_frontier,
            &mut self.validation_visited,
            &mut |pos, _, _| found.push(pos),
            false,
        );
        self.new_validation_set.extend(found);
        if result.is_err() {
            self.validation_frontier.clear();
            self.validation_visited.clear();
            self.base.revalidate_in = cfg.validation_timer() * 2;
            return;
        }
        if self.validation_visited.len() as i64 >= i64::from(cfg.max_blocks)
            && cfg.can_drain_infinitely(self.fluid.as_deref())
        {
            if !self.base.infinite {
                self.reset(cfg);
            }
            self.validation_frontier.clear();
            self.base.revalidate_in = cfg.validation_timer() * 2;
            return;
        }
        if !self.validation_frontier.is_empty() {
            return;
        }
        if self.base.infinite {
            self.reset(cfg);
            return;
        }
        self.validation_set = std::mem::take(&mut self.new_validation_set);
        self.validation_visited.clear();
    }

    /// `getDrainableFluid`: the fluid a bucket's worth can be taken of now, without taking it. Unlike
    /// Create's it always looks (`pullNext` in simulation), which is what a `drain` of the handler
    /// does first: a hose that knows no fluid yet only learns it that way, and a network asks what a
    /// handler holds before it asks it to give.
    pub fn drainable(&mut self, level: &dyn Level, cfg: &Config, root: Pos) -> Option<String> {
        if self.searching() {
            return None;
        }
        if !self.pull_next(&mut ReadOnly(level), cfg, root, true) {
            return None;
        }
        self.fluid.clone()
    }

    /// `pullNext`: takes (or, `simulate`, checks that it could take) the next bucket of the body.
    pub fn pull_next(&mut self, level: &mut dyn Edit, cfg: &Config, root: Pos, simulate: bool) -> bool {
        if !self.base.frontier.is_empty() {
            return false;
        }
        if self.base.root != Some(root) {
            self.rebuild_context(cfg, root);
            return false;
        }
        if self.base.counterpart_acted {
            self.base.counterpart_acted = false;
            self.soft_reset(cfg, root);
            return false;
        }
        if !self.queue.is_empty() && !self.is_valid {
            self.rebuild_context(cfg, root);
            return false;
        }
        if self.validation_frontier.is_empty() && !self.queue.is_empty() && !simulate && self.base.revalidate_in == 0 {
            self.revalidate(cfg, root);
        }
        if self.base.infinite {
            if !simulate && let Some(fluid) = self.fluid.clone() {
                level.effect(root, &fluid, true);
            }
            return true;
        }
        if simulate {
            // Looks down the queue without changing it: the first source of the fluid.
            let held = self.fluid.clone();
            let found = self.queue.iter().any(|(current, _)| {
                let cell = level.cell(current);
                matches!(cell.fluid_state(), Some((fluid, true)) if Some(fluid) == held.as_deref())
            });
            if found {
                return true;
            }
        }
        while !simulate && let Some((current, _)) = self.queue.first() {
            let cell = level.cell(current);
            let mut fluid: Option<String> = None;
            let mut emptied = Change::Air;
            match &cell {
                Cell::Waterlogged(true) => {
                    emptied = Change::Unwaterlog;
                    fluid = Some("minecraft:water".to_owned());
                }
                Cell::Liquid { fluid: held, level: height } => {
                    if *height == 0 {
                        fluid = Some(held.clone());
                    } else {
                        // A flowing cell goes with no bucket.
                        if !simulate {
                            level.set(current, Change::Air);
                        }
                        self.queue.dequeue();
                        if self.queue.is_empty() {
                            self.is_valid = self.check_valid(level, self.base.root.unwrap_or(root));
                            self.reset(cfg);
                        }
                        continue;
                    }
                }
                Cell::Wet { fluid: held } => fluid = Some(held.clone()),
                _ => {}
            }
            if self.fluid.is_none() {
                self.fluid.clone_from(&fluid);
            }
            if fluid.is_none() || self.fluid != fluid {
                self.queue.dequeue();
                if self.queue.is_empty() {
                    self.is_valid = self.check_valid(level, self.base.root.unwrap_or(root));
                    self.reset(cfg);
                }
                continue;
            }
            if simulate {
                return true;
            }
            let held = fluid.unwrap_or_default();
            level.effect(current, &held, true);
            level.set(current, emptied);
            self.queue.dequeue();
            if self.queue.is_empty() {
                self.is_valid = self.check_valid(level, self.base.root.unwrap_or(root));
                self.reset(cfg);
            } else if !self.validation_set.contains(&current) {
                self.reset(cfg);
            }
            return true;
        }
        if self.base.root.is_none() {
            return false;
        }
        if self.is_valid {
            self.rebuild_context(cfg, root);
        }
        false
    }
}

/// A [`Level`] that can't be changed, for the checks that only look.
struct ReadOnly<'a>(&'a dyn Level);

impl Level for ReadOnly<'_> {
    fn loaded(&self, pos: Pos) -> bool {
        self.0.loaded(pos)
    }
    fn cell(&self, pos: Pos) -> Cell {
        self.0.cell(pos)
    }
    fn ultra_warm(&self) -> bool {
        self.0.ultra_warm()
    }
}

impl Edit for ReadOnly<'_> {
    fn set(&mut self, _: Pos, _: Change) {}
    fn effect(&mut self, _: Pos, _: &str, _: bool) {}
    fn evaporates(&mut self, _: Pos) {}
}

// --- filling --------------------------------------------------------------------------------------

/// `FluidFillingBehaviour.SpaceType`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Space {
    Fillable,
    Filled,
    Blocking,
}

/// `FluidFillingBehaviour.getAtPos`.
fn space_of(cell: &Cell, to_fill: &str) -> Space {
    match cell {
        Cell::Waterlogged(logged) => {
            if to_fill == "minecraft:water" {
                if *logged { Space::Filled } else { Space::Fillable }
            } else {
                Space::Blocking
            }
        }
        Cell::Liquid { fluid, level } => {
            if *level == 0 {
                if fluid == to_fill { Space::Filled } else { Space::Blocking }
            } else {
                Space::Fillable
            }
        }
        Cell::Wet { fluid } => {
            if fluid == to_fill { Space::Filled } else { Space::Blocking }
        }
        Cell::Other { replaceable_by_fluid } => {
            if *replaceable_by_fluid { Space::Fillable } else { Space::Blocking }
        }
    }
}

/// `FluidFillingBehaviour`: places a bucket of a fluid at a time in the space the hose's end is
/// in, the lowest cells first, the nearest first, spreading over connected space.
#[derive(Clone, Debug)]
pub struct Filler {
    base: Base,
    queue: Ranked,
    infinity_check_frontier: VecDeque<(Pos, i32)>,
    infinity_check_visited: HashSet<Pos>,
    pub dirty: bool,
}

impl Filler {
    pub fn new(cfg: &Config) -> Self {
        let mut base = Base::new(cfg);
        base.revalidate_in = 1;
        Filler {
            base,
            queue: Ranked::fill_order(),
            infinity_check_frontier: VecDeque::new(),
            infinity_check_visited: HashSet::new(),
            dirty: false,
        }
    }

    pub fn infinite(&self) -> bool {
        self.base.infinite
    }

    pub fn counterpart_acted(&mut self) {
        self.base.counterpart_acted = true;
    }

    pub fn reset(&mut self, cfg: &Config) {
        self.base.reset(cfg);
        self.queue.clear();
        self.infinity_check_frontier.clear();
        self.infinity_check_visited.clear();
        self.dirty = true;
    }

    fn soft_reset(&mut self, cfg: &Config, root: Pos) {
        self.base.visited.clear();
        self.queue.clear();
        self.queue.enqueue(root, 0, root);
        self.base.infinite = false;
        self.base.revalidate_in = cfg.validation_timer();
        self.dirty = true;
    }

    /// `FluidFillingBehaviour.tick`.
    pub fn tick(&mut self, level: &dyn Level, cfg: &Config) {
        if !self.infinity_check_frontier.is_empty()
            && let Some(root) = self.base.root
            && let Some((fluid, _)) = level.cell(root).fluid_state()
        {
            let fluid = fluid.to_owned();
            self.continue_validation(level, cfg, &fluid, root);
        }
        if self.base.revalidate_in > 0 {
            self.base.revalidate_in -= 1;
        }
    }

    fn continue_validation(&mut self, level: &dyn Level, cfg: &Config, fluid: &str, root: Pos) {
        let result = search(
            level,
            cfg,
            root,
            Some(fluid),
            &mut self.infinity_check_frontier,
            &mut self.infinity_check_visited,
            // Create's callback puts each kept cell back on the frontier it is walking.
            &mut |pos, distance, frontier| frontier.push_back((pos, distance)),
            true,
        );
        if result.is_err() {
            self.infinity_check_frontier.clear();
            self.infinity_check_visited.clear();
            self.base.revalidate_in = cfg.validation_timer() * 2;
            return;
        }
        if self.infinity_check_visited.len() as i64 >= i64::from(cfg.max_blocks)
            && cfg.max_blocks != -1
            && !cfg.fill_infinite
        {
            if !self.base.infinite {
                self.reset(cfg);
                self.base.infinite = true;
                self.dirty = true;
            }
            self.infinity_check_frontier.clear();
            self.base.revalidate_in = cfg.validation_timer() * 2;
            return;
        }
        if !self.infinity_check_frontier.is_empty() {
            return;
        }
        if self.base.infinite {
            self.reset(cfg);
            return;
        }
        self.infinity_check_visited.clear();
    }

    /// `tryDeposit`: places a bucket of `fluid` in the space at `root` (or, `simulate`, checks that
    /// it could).
    pub fn try_deposit(&mut self, level: &mut dyn Edit, cfg: &Config, fluid: &str, root: Pos, simulate: bool) -> bool {
        if self.base.root != Some(root) {
            self.reset(cfg);
            self.base.root = Some(root);
            self.queue.enqueue(root, 0, root);
            return false;
        }
        if self.base.counterpart_acted {
            self.base.counterpart_acted = false;
            self.soft_reset(cfg, root);
            return false;
        }
        if self.base.revalidate_in == 0 {
            self.base.visited.clear();
            self.infinity_check_frontier.clear();
            self.infinity_check_visited.clear();
            self.infinity_check_frontier.push_back((root, 0));
            self.base.revalidate_in = cfg.validation_timer();
            self.soft_reset(cfg, root);
        }
        let range_sq = i64::from(cfg.range) * i64::from(cfg.range);
        let evaporate = level.ultra_warm() && fluid == "minecraft:water";
        let can_place_sources = cfg.place_sources;

        if (!cfg.fill_infinite && self.base.infinite) || evaporate || !can_place_sources {
            let equivalent = level.cell(root).fluid_state().is_some_and(|(held, _)| held == fluid);
            if !equivalent && !evaporate && can_place_sources {
                return false;
            }
            if simulate {
                return true;
            }
            level.effect(root, fluid, false);
            if evaporate {
                level.evaporates(root);
            }
            return true;
        }

        let mut success = false;
        let mut i = 0;
        while !success && !self.queue.is_empty() && i < SEARCHED_PER_TICK {
            i += 1;
            let Some((current, distance)) = self.queue.first() else { break };
            if self.base.visited.contains(&current) {
                self.queue.dequeue();
                continue;
            }
            if !simulate {
                self.base.visited.insert(current);
            }
            if self.base.visited.len() as i64 >= i64::from(cfg.max_blocks) && cfg.max_blocks != -1 {
                self.base.infinite = true;
                if !cfg.fill_infinite {
                    self.base.visited.clear();
                    self.queue.clear();
                    return false;
                }
            }
            let cell = level.cell(current);
            let space = space_of(&cell, fluid);
            if space == Space::Blocking {
                continue;
            }
            if space == Space::Fillable {
                success = true;
                if !simulate {
                    level.effect(current, fluid, false);
                    let change = if matches!(cell, Cell::Waterlogged(_)) && fluid == "minecraft:water" {
                        Change::Waterlog
                    } else {
                        Change::Source(fluid.to_owned())
                    };
                    level.set(current, change);
                }
            }
            if simulate && success {
                return true;
            }
            self.base.visited.insert(current);
            self.queue.dequeue();
            for side in DIRECTIONS {
                if side == Dir::Up {
                    continue;
                }
                let offset = relative(current, side);
                if self.base.visited.contains(&offset) {
                    continue;
                }
                if dist_sq(offset, root) > range_sq {
                    continue;
                }
                if space_of(&level.cell(offset), fluid) != Space::Blocking {
                    self.queue.enqueue(offset, distance + 1, root);
                }
            }
        }
        success
    }
}

// --- the hose's own tank and what it does with fluid ---------------------------------------------------

/// `internalTank`: a tank of [`TANK_CAPACITY`] holding one fluid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tank {
    pub fluid: Option<String>,
    pub amount: i32,
}

impl Tank {
    pub fn is_empty(&self) -> bool {
        self.fluid.is_none() || self.amount <= 0
    }

    /// `FluidTank.fill`: how much went in.
    pub fn fill(&mut self, fluid: &str, amount: i32, execute: bool) -> i32 {
        if amount <= 0 || !self.is_empty() && self.fluid.as_deref() != Some(fluid) {
            return 0;
        }
        let room = TANK_CAPACITY - if self.is_empty() { 0 } else { self.amount };
        let taken = amount.min(room).max(0);
        if execute && taken > 0 {
            self.fluid = Some(fluid.to_owned());
            self.amount = if self.is_empty() { taken } else { self.amount + taken };
        }
        taken
    }

    /// `FluidTank.drain` of up to `max`: the fluid and the amount.
    pub fn drain(&mut self, max: i32, execute: bool) -> Option<(String, i32)> {
        if self.is_empty() || max <= 0 {
            return None;
        }
        let taken = max.min(self.amount);
        let fluid = self.fluid.clone()?;
        if execute {
            self.amount -= taken;
            if self.amount <= 0 {
                *self = Tank::default();
            }
        }
        Some((fluid, taken))
    }
}

/// `HosePulleyFluidHandler.fill`: how much of `resource` the hose takes. `moving` is the hose's
/// `isMoving` (a moving hose deposits nothing). The deposit goes into the world at `root`.
#[allow(clippy::too_many_arguments)]
pub fn handler_fill(
    tank: &mut Tank,
    drainer: &mut Drainer,
    filler: &mut Filler,
    level: &mut dyn Edit,
    cfg: &Config,
    root: Pos,
    moving: bool,
    fluid: &str,
    amount: i32,
    execute: bool,
) -> i32 {
    if !tank.is_empty() && tank.fluid.as_deref() != Some(fluid) {
        return 0;
    }
    if amount <= 0 || !has_block_state(fluid) {
        return 0;
    }
    let mut diff = amount;
    let total_after = diff + if tank.is_empty() { 0 } else { tank.amount };
    let mut remaining = amount;
    let mut deposited = false;
    if !moving && total_after >= BUCKET && filler.try_deposit(level, cfg, fluid, root, !execute) {
        drainer.counterpart_acted();
        remaining -= BUCKET;
        diff -= BUCKET;
        deposited = true;
    }
    if !execute {
        return if diff <= 0 { amount } else { tank.fill(fluid, remaining, false) };
    }
    if diff <= 0 {
        tank.drain(-diff, true);
        return amount;
    }
    tank.fill(fluid, remaining, true) + if deposited { BUCKET } else { 0 }
}

/// `HosePulleyFluidHandler.drainInternal`: the fluid and amount the hose gives of up to `max`
/// (of `only`, if a fluid was asked for).
#[allow(clippy::too_many_arguments)]
pub fn handler_drain(
    tank: &mut Tank,
    drainer: &mut Drainer,
    filler: &mut Filler,
    level: &mut dyn Edit,
    cfg: &Config,
    root: Pos,
    moving: bool,
    max: i32,
    only: Option<&str>,
    execute: bool,
) -> Option<(String, i32)> {
    if let Some(only) = only
        && !tank.is_empty()
        && tank.fluid.as_deref() != Some(only)
    {
        return None;
    }
    if tank.amount >= BUCKET && !tank.is_empty() {
        return tank.drain(max, execute);
    }
    let returned = drainer.drainable(&ReadOnly(level), cfg, root);
    if moving || !drainer.pull_next(level, cfg, root, !execute) {
        return tank.drain(max, execute);
    }
    filler.counterpart_acted();
    let available = BUCKET + if tank.is_empty() { 0 } else { tank.amount };
    let Some(returned) = returned else {
        return tank.drain(max, execute);
    };
    if !tank.is_empty() && tank.fluid.as_deref() != Some(returned.as_str()) {
        return tank.drain(max, execute);
    }
    if let Some(only) = only
        && only != returned
    {
        return None;
    }
    let drained = max.min(available);
    let leftover = available - drained;
    if execute && leftover > 0 {
        *tank = Tank { fluid: Some(returned.clone()), amount: leftover };
    } else if execute {
        *tank = Tank::default();
    }
    Some((returned, drained))
}

/// What the capability shows through its single tank: the internal tank's fluid, or (when it is
/// empty) a bucket of what the hose could drain (`getFluidInTank`).
pub fn shown(tank: &Tank, drainable: Option<String>) -> Option<(String, i32)> {
    if !tank.is_empty() {
        return tank.fluid.clone().map(|f| (f, tank.amount));
    }
    drainable.map(|f| (f, BUCKET))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Grid {
        cells: HashMap<Pos, Cell>,
        edits: Vec<(Pos, Change)>,
        floor: i32,
        hot: bool,
    }

    impl Grid {
        fn water_box(min: Pos, max: Pos) -> Grid {
            let mut grid = Grid { floor: min[1] - 1, ..Default::default() };
            for x in min[0]..=max[0] {
                for y in min[1]..=max[1] {
                    for z in min[2]..=max[2] {
                        grid.cells.insert([x, y, z], Cell::Liquid { fluid: "minecraft:water".into(), level: 0 });
                    }
                }
            }
            grid
        }
    }

    impl Level for Grid {
        fn loaded(&self, _: Pos) -> bool {
            true
        }
        fn cell(&self, pos: Pos) -> Cell {
            if let Some(cell) = self.cells.get(&pos) {
                return cell.clone();
            }
            Cell::Other { replaceable_by_fluid: pos[1] > self.floor }
        }
        fn ultra_warm(&self) -> bool {
            self.hot
        }
    }

    impl Edit for Grid {
        fn set(&mut self, pos: Pos, change: Change) {
            match &change {
                Change::Air => {
                    self.cells.remove(&pos);
                }
                Change::Unwaterlog => {
                    self.cells.insert(pos, Cell::Waterlogged(false));
                }
                Change::Waterlog => {
                    self.cells.insert(pos, Cell::Waterlogged(true));
                }
                Change::Source(fluid) => {
                    self.cells.insert(pos, Cell::Liquid { fluid: fluid.clone(), level: 0 });
                }
            }
            self.edits.push((pos, change));
        }
        fn effect(&mut self, _: Pos, _: &str, _: bool) {}
        fn evaporates(&mut self, _: Pos) {}
    }

    const OWN: Pos = [0, 10, 0];
    const ROOT: Pos = [0, 4, 0];

    /// Runs the drainer's ticks until it has finished searching.
    fn settle(drainer: &mut Drainer, grid: &Grid, cfg: &Config) {
        for _ in 0..400 {
            drainer.tick(grid, cfg);
        }
    }

    /// Create's dance before a hose can take anything: a pull builds the context, a tick finds
    /// the cell valid, the next pull starts the search, and ticks run it.
    fn warm(drainer: &mut Drainer, grid: &mut Grid, cfg: &Config, root: Pos) {
        for _ in 0..4 {
            drainer.pull_next(grid, cfg, root, true);
            settle(drainer, grid, cfg);
        }
    }

    /// Takes buckets until none comes, lowering the hose's end as the fluid goes (the root is
    /// where the hose hangs): how many were taken.
    fn pull_all(drainer: &mut Drainer, grid: &mut Grid, cfg: &Config, root: Pos, most: usize) -> usize {
        let mut root = root;
        let mut taken = 0;
        let mut idle = 0;
        while taken < most && idle < 6 {
            if drainer.pull_next(grid, cfg, root, false) {
                taken += 1;
                idle = 0;
                continue;
            }
            idle += 1;
            if matches!(grid.cell(root), Cell::Other { .. }) && root[1] > 1 {
                root[1] -= 1;
            }
            settle(drainer, grid, cfg);
        }
        taken
    }

    #[test]
    fn the_hose_lets_down_at_the_linear_speed_and_stops_on_a_solid_block() {
        assert!((movement_speed(512.0) - 1.0).abs() < 1e-6);
        let free = |depth: i32| depth < 5;
        // 4.9 plus 0.2 would reach into depth 6's block.
        let (offset, moving) = tick(4.9, true, 0.2, false, &free);
        assert_eq!(offset, 5.0);
        assert!(!moving);
        // Reeling in stops at the pulley.
        let (offset, moving) = tick(0.1, true, -0.3, false, &|_| true);
        assert_eq!(offset, 0.0);
        assert!(!moving);
        // No speed, no motion.
        assert!(!tick(2.0, true, 0.0, true, &|_| true).1);
        assert_eq!(end_depth(2.01), 3);
    }

    #[test]
    fn a_speed_change_rounds_a_stopped_hose_and_starts_the_searches_of_a_moving_one() {
        let (offset, moving, reset) = speed_changed(2.6, 0.0, &|_| true);
        assert_eq!((offset, moving, reset), (3.0, false, false));
        let (_, moving, reset) = speed_changed(2.0, 64.0, &|_| true);
        assert!(moving && reset);
        // Blocked below: doesn't move.
        let (_, moving, _) = speed_changed(2.0, 64.0, &|depth| depth < 3);
        assert!(!moving);
        // Reeling in past the pulley doesn't move.
        let (_, moving, _) = speed_changed(0.0, -64.0, &|_| true);
        assert!(!moving);
    }

    #[test]
    fn a_blocked_hose_restarts_when_the_space_below_opens_and_it_is_let_down() {
        assert!(restarts(2.0, 64.0, &|_| true));
        assert!(!restarts(2.0, -64.0, &|_| true));
        assert!(!restarts(2.0, 64.0, &|_| false));
    }

    #[test]
    fn the_client_follows_a_moving_hose_from_a_snapshot() {
        let motion = Motion { offset: 1.0, time: 100, moving: true };
        assert!((motion.offset_at(512.0, 110.0) - 11.0).abs() < 1e-4);
        assert_eq!(motion.offset_at(-512.0, 110.0), 0.0);
        let still = Motion { moving: false, ..motion };
        assert_eq!(still.offset_at(512.0, 500.0), 1.0);
        assert_eq!(drawn_offset(0.0), MIN_DRAWN_OFFSET);
        assert_eq!(drawn_offset(2.5), 2.5);
    }

    #[test]
    fn the_queue_takes_the_highest_then_the_farthest_cell_for_a_drain_and_the_reverse_for_a_fill() {
        let mut drain = Ranked::drain_order();
        let mut fill = Ranked::fill_order();
        for (pos, distance) in [([0, 1, 0], 0), ([0, 2, 0], 1), ([3, 2, 0], 4), ([1, 2, 0], 2)] {
            drain.enqueue(pos, distance, [0, 1, 0]);
            fill.enqueue(pos, distance, [0, 1, 0]);
        }
        // Drain: y = 2 first, and of those the greatest distance.
        assert_eq!(drain.first(), Some(([3, 2, 0], 4)));
        drain.dequeue();
        assert_eq!(drain.first(), Some(([1, 2, 0], 2)));
        // Fill: the lowest first.
        assert_eq!(fill.first(), Some(([0, 1, 0], 0)));
        fill.dequeue();
        assert_eq!(fill.first(), Some(([0, 2, 0], 1)));
    }

    #[test]
    fn a_small_pool_is_drained_a_bucket_at_a_time_from_the_top_down_and_runs_out() {
        let cfg = Config::default();
        let mut grid = Grid::water_box([-1, 1, -1], [1, 4, 1]); // 3 x 4 x 3 = 36 sources
        let mut drainer = Drainer::new(OWN, &cfg);
        warm(&mut drainer, &mut grid, &cfg, ROOT);
        assert!(!drainer.infinite());
        let taken = pull_all(&mut drainer, &mut grid, &cfg, ROOT, 100);
        assert_eq!(taken, 36);
        // Highest cells went first.
        assert!(grid.edits.iter().take(9).all(|(pos, _)| pos[1] == 4));
        assert!(grid.cells.is_empty());
    }

    #[test]
    fn a_body_of_the_threshold_size_is_infinite_and_never_removed() {
        // The search goes up and sideways from the hose's end, so it is the layer at the end that
        // counts: 7 x 7 = 49 cells at a threshold of 40.
        let cfg = Config { max_blocks: 40, ..Config::default() };
        let mut grid = Grid::water_box([-3, 1, -3], [3, 4, 3]);
        let mut drainer = Drainer::new(OWN, &cfg);
        warm(&mut drainer, &mut grid, &cfg, ROOT);
        assert!(drainer.infinite());
        let before = grid.cells.len();
        for _ in 0..50 {
            assert!(drainer.pull_next(&mut grid, &cfg, ROOT, false));
        }
        assert_eq!(grid.cells.len(), before);
        assert!(grid.edits.is_empty());
    }

    #[test]
    fn a_fluid_that_isnt_bottomless_is_drained_however_big_the_body() {
        let cfg = Config { max_blocks: 10, ..Config::default() };
        let mut grid = Grid::water_box([-2, 1, -2], [2, 2, 2]);
        for cell in grid.cells.values_mut() {
            *cell = Cell::Liquid { fluid: "create:honey".into(), level: 0 };
        }
        let mut drainer = Drainer::new(OWN, &cfg);
        warm(&mut drainer, &mut grid, &cfg, [0, 2, 0]);
        assert!(!drainer.infinite());
        assert!(drainer.pull_next(&mut grid, &cfg, [0, 2, 0], false));
        assert_eq!(grid.edits.len(), 1);
    }

    #[test]
    fn a_threshold_of_minus_one_turns_the_infinite_source_off() {
        let cfg = Config { max_blocks: -1, ..Config::default() };
        assert!(!cfg.can_drain_infinitely(Some("minecraft:water")));
        assert!(Config::default().can_drain_infinitely(Some("minecraft:lava")));
        assert!(!Config::default().can_drain_infinitely(Some("create:honey")));
        assert!(!Config::default().can_drain_infinitely(None));
    }

    #[test]
    fn flowing_water_goes_without_a_bucket_and_a_waterlogged_block_gives_water() {
        let cfg = Config::default();
        let mut grid = Grid::default();
        grid.cells.insert([0, 4, 0], Cell::Liquid { fluid: "minecraft:water".into(), level: 3 });
        grid.cells.insert([0, 3, 0], Cell::Waterlogged(true));
        grid.cells.insert([1, 3, 0], Cell::Liquid { fluid: "minecraft:water".into(), level: 0 });
        let mut drainer = Drainer::new(OWN, &cfg);
        let root = [0, 3, 0];
        warm(&mut drainer, &mut grid, &cfg, root);
        // The highest cell is the flowing one above the hose's end: it goes with no bucket, and
        // the same call goes on to a bucket out of a source.
        assert!(drainer.pull_next(&mut grid, &cfg, root, false));
        assert_eq!(grid.edits[0], ([0, 4, 0], Change::Air));
        assert!(drainer.pull_next(&mut grid, &cfg, root, false));
        assert!(grid.edits.iter().any(|(pos, change)| *pos == [0, 3, 0] && *change == Change::Unwaterlog)
            || grid.edits.iter().any(|(pos, change)| *pos == [1, 3, 0] && *change == Change::Air));
    }

    #[test]
    fn asking_what_can_be_drained_starts_the_search_of_a_hose_that_knows_nothing_yet() {
        let cfg = Config::default();
        let mut grid = Grid::water_box([-1, 1, -1], [1, 3, 1]);
        let mut drainer = Drainer::new(OWN, &cfg);
        let root = [0, 3, 0];
        // Only asking, as a pump does before it asks for fluid: the answers are nothing until the
        // context is built and searched, then a bucket of water.
        let mut answer = None;
        for _ in 0..6 {
            answer = drainer.drainable(&grid, &cfg, root);
            if answer.is_some() {
                break;
            }
            settle(&mut drainer, &grid, &cfg);
        }
        assert_eq!(answer.as_deref(), Some("minecraft:water"));
        assert!(grid.edits.is_empty());
        let _ = &mut grid;
    }

    #[test]
    fn a_hose_hanging_in_air_has_nothing_to_drain() {
        let cfg = Config::default();
        let mut grid = Grid::default();
        let mut drainer = Drainer::new(OWN, &cfg);
        assert!(!drainer.pull_next(&mut grid, &cfg, ROOT, false));
        settle(&mut drainer, &grid, &cfg);
        assert!(!drainer.pull_next(&mut grid, &cfg, ROOT, false));
        assert!(drainer.drainable(&grid, &cfg, ROOT).is_none());
    }

    #[test]
    fn the_search_stays_in_range_and_follows_flowing_fluid_up_only_in_the_hoses_column() {
        let cfg = Config { range: 3, ..Config::default() };
        let mut grid = Grid::water_box([-10, 1, -10], [10, 1, 10]);
        let mut frontier = VecDeque::from([([0, 1, 0], 0)]);
        let mut visited = HashSet::new();
        let mut found = Vec::new();
        let _ = search(&grid, &cfg, [0, 1, 0], None, &mut frontier, &mut visited, &mut |p, _, _| found.push(p), false);
        // Within 3 blocks of the root: the disc of radius 3 has 29 cells.
        assert_eq!(found.len(), 29);
        // Flowing water above: not followed off the column.
        grid.cells.insert([1, 2, 0], Cell::Liquid { fluid: "minecraft:water".into(), level: 2 });
        let mut frontier = VecDeque::from([([0, 1, 0], 0)]);
        let mut visited = HashSet::new();
        let mut found = Vec::new();
        let _ = search(&grid, &cfg, [0, 1, 0], None, &mut frontier, &mut visited, &mut |p, _, _| found.push(p), false);
        // Flowing water above a neighbour of the column is not followed.
        assert!(!found.contains(&[1, 2, 0]));
        grid.cells.insert([0, 2, 0], Cell::Liquid { fluid: "minecraft:water".into(), level: 2 });
        let mut frontier = VecDeque::from([([0, 1, 0], 0)]);
        let mut visited = HashSet::new();
        let mut found = Vec::new();
        let _ = search(&grid, &cfg, [0, 1, 0], None, &mut frontier, &mut visited, &mut |p, _, _| found.push(p), false);
        // Straight up the hose's own column it is.
        assert!(found.contains(&[0, 2, 0]));
    }

    #[test]
    fn a_fill_places_a_source_in_the_lowest_free_cell_beside_the_hose_and_spreads() {
        let cfg = Config::default();
        let mut grid = Grid { floor: 0, ..Default::default() };
        let mut filler = Filler::new(&cfg);
        // First call: sets the root up.
        assert!(!filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 1, 0], false));
        assert!(filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 1, 0], false));
        assert_eq!(grid.edits[0].0, [0, 1, 0]);
        // The next goes beside the first, not above it.
        assert!(filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 1, 0], false));
        assert_eq!(grid.edits[1].0[1], 1);
        // A simulation places nothing.
        let n = grid.edits.len();
        assert!(filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 1, 0], true));
        assert_eq!(grid.edits.len(), n);
    }

    #[test]
    fn a_fill_of_another_fluid_is_blocked_by_a_source_and_a_waterlogged_block_takes_only_water() {
        assert_eq!(space_of(&Cell::Liquid { fluid: "minecraft:lava".into(), level: 0 }, "minecraft:water"), Space::Blocking);
        assert_eq!(space_of(&Cell::Liquid { fluid: "minecraft:lava".into(), level: 4 }, "minecraft:water"), Space::Fillable);
        assert_eq!(space_of(&Cell::Waterlogged(false), "minecraft:water"), Space::Fillable);
        assert_eq!(space_of(&Cell::Waterlogged(false), "minecraft:lava"), Space::Blocking);
        assert_eq!(space_of(&Cell::Waterlogged(true), "minecraft:water"), Space::Filled);
        assert_eq!(space_of(&Cell::Other { replaceable_by_fluid: false }, "minecraft:water"), Space::Blocking);
    }

    #[test]
    fn in_the_nether_water_evaporates_instead_of_being_placed() {
        let cfg = Config::default();
        let mut grid = Grid { floor: 0, hot: true, ..Default::default() };
        let mut filler = Filler::new(&cfg);
        filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 1, 0], false);
        assert!(filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 1, 0], false));
        assert!(grid.edits.is_empty());
    }

    #[test]
    fn without_source_placement_a_fill_only_succeeds_into_the_same_fluid() {
        let cfg = Config { place_sources: false, ..Config::default() };
        let mut grid = Grid::water_box([0, 1, 0], [0, 1, 0]);
        let mut filler = Filler::new(&cfg);
        filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 1, 0], false);
        assert!(filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 1, 0], false));
        assert!(grid.edits.is_empty());
    }

    #[test]
    fn a_fill_into_a_body_of_the_threshold_size_is_voided_unless_fill_infinite_is_on() {
        let cfg = Config { max_blocks: 30, ..Config::default() };
        let mut grid = Grid::water_box([-3, 1, -3], [3, 2, 3]); // 98 sources
        let mut filler = Filler::new(&cfg);
        filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 2, 0], false);
        // `revalidateIn` starts at 1: the second try begins the infinity check, which then runs on ticks.
        filler.tick(&grid, &cfg);
        filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 2, 0], false);
        for _ in 0..40 {
            filler.tick(&grid, &cfg);
        }
        assert!(filler.infinite());
        let edits = grid.edits.len();
        assert!(filler.try_deposit(&mut grid, &cfg, "minecraft:water", [0, 2, 0], false));
        assert_eq!(grid.edits.len(), edits);
    }

    #[test]
    fn the_handler_deposits_a_bucket_once_a_bucket_is_in_and_keeps_the_rest_in_its_tank() {
        let cfg = Config::default();
        let mut grid = Grid { floor: 0, ..Default::default() };
        let mut tank = Tank::default();
        let mut drainer = Drainer::new(OWN, &cfg);
        let mut filler = Filler::new(&cfg);
        let mut fill = |tank: &mut Tank, drainer: &mut Drainer, filler: &mut Filler, grid: &mut Grid, amount, execute| {
            handler_fill(tank, drainer, filler, grid, &cfg, [0, 1, 0], false, "minecraft:water", amount, execute)
        };
        // Under a bucket in all: kept in the tank.
        assert_eq!(fill(&mut tank, &mut drainer, &mut filler, &mut grid, 600, true), 600);
        assert_eq!(tank.amount, 600);
        // 1200 in all asks for a deposit, whose first try only sets the filler's root up.
        assert_eq!(fill(&mut tank, &mut drainer, &mut filler, &mut grid, 600, true), 600);
        assert_eq!(tank.amount, 1200);
        assert!(grid.edits.is_empty());
        // Now 1500 in all: a bucket is deposited and the tank keeps what is over.
        assert_eq!(fill(&mut tank, &mut drainer, &mut filler, &mut grid, 300, true), 300);
        assert_eq!(tank.amount, 500);
        assert_eq!(grid.edits.len(), 1);
        // A simulation promises the same and changes nothing.
        assert_eq!(fill(&mut tank, &mut drainer, &mut filler, &mut grid, 600, false), 600);
        assert_eq!((tank.amount, grid.edits.len()), (500, 1));
        // Another fluid is refused while the tank holds water.
        assert_eq!(
            handler_fill(&mut tank, &mut drainer, &mut filler, &mut grid, &cfg, [0, 1, 0], false, "minecraft:lava", 100, true),
            0
        );
        // A fluid with no block (tea) is refused.
        let mut empty = Tank::default();
        assert_eq!(
            handler_fill(&mut empty, &mut drainer, &mut filler, &mut grid, &cfg, [0, 1, 0], false, "create:tea", 1000, true),
            0
        );
    }

    #[test]
    fn a_moving_hose_deposits_and_pulls_nothing() {
        let cfg = Config::default();
        let mut grid = Grid { floor: 0, ..Default::default() };
        let mut tank = Tank::default();
        let mut drainer = Drainer::new(OWN, &cfg);
        let mut filler = Filler::new(&cfg);
        let took = handler_fill(&mut tank, &mut drainer, &mut filler, &mut grid, &cfg, [0, 1, 0], true, "minecraft:water", 1000, true);
        assert_eq!(took, 1000);
        assert_eq!(tank.amount, 1000);
        assert!(grid.edits.is_empty());
    }

    #[test]
    fn the_handler_drains_a_bucket_out_of_the_world_and_gives_what_was_asked() {
        let cfg = Config::default();
        let mut grid = Grid::water_box([-1, 1, -1], [1, 3, 1]);
        let mut tank = Tank::default();
        let mut drainer = Drainer::new(OWN, &cfg);
        let mut filler = Filler::new(&cfg);
        let root = [0, 3, 0];
        warm(&mut drainer, &mut grid, &cfg, root);
        assert_eq!(drainer.drainable(&grid, &cfg, root).as_deref(), Some("minecraft:water"));
        // A simulation takes nothing out of the world.
        let looked = handler_drain(&mut tank, &mut drainer, &mut filler, &mut grid, &cfg, root, false, 400, None, false);
        assert_eq!(looked, Some(("minecraft:water".to_owned(), 400)));
        assert!(grid.edits.is_empty());
        let got = handler_drain(&mut tank, &mut drainer, &mut filler, &mut grid, &cfg, root, false, 400, None, true);
        assert_eq!(got, Some(("minecraft:water".to_owned(), 400)));
        // The bucket's remainder stays in the tank.
        assert_eq!(tank.amount, 600);
        assert_eq!(grid.edits.len(), 1);
        // Asking for another fluid gets nothing.
        assert_eq!(
            handler_drain(&mut tank, &mut drainer, &mut filler, &mut grid, &cfg, root, false, 100, Some("minecraft:lava"), true),
            None
        );
        // A moving hose gives only what its tank holds.
        let moving = handler_drain(&mut tank, &mut drainer, &mut filler, &mut grid, &cfg, root, true, 5000, None, true);
        assert_eq!(moving, Some(("minecraft:water".to_owned(), 600)));
        assert_eq!(grid.edits.len(), 1);
    }

    #[test]
    fn what_the_capability_shows_is_the_tank_or_a_bucket_of_what_could_be_drained() {
        assert_eq!(shown(&Tank::default(), None), None);
        assert_eq!(shown(&Tank::default(), Some("minecraft:lava".into())), Some(("minecraft:lava".into(), BUCKET)));
        let held = Tank { fluid: Some("minecraft:water".into()), amount: 250 };
        assert_eq!(shown(&held, Some("minecraft:lava".into())), Some(("minecraft:water".into(), 250)));
    }

    #[test]
    fn the_validation_timer_waits_for_the_threshold_to_be_reachable() {
        assert_eq!(Config::default().validation_timer(), 160);
        assert_eq!(Config { max_blocks: 1_000_000, ..Config::default() }.validation_timer(), 977);
        assert_eq!(Config { max_blocks: -1, ..Config::default() }.validation_timer(), 160);
    }

    #[test]
    fn the_tank_fills_to_its_capacity_and_drains_back() {
        let mut tank = Tank::default();
        assert_eq!(tank.fill("minecraft:water", 2000, true), 1500);
        assert_eq!(tank.fill("minecraft:lava", 10, true), 0);
        assert_eq!(tank.drain(500, true), Some(("minecraft:water".into(), 500)));
        assert_eq!(tank.amount, 1000);
        assert_eq!(tank.drain(5000, true), Some(("minecraft:water".into(), 1000)));
        assert!(tank.is_empty());
    }
}
