//! The Hose Pulley (`HosePulleyBlock`, `HosePulleyBlockEntity`, `HosePulleyFluidHandler`,
//! `FluidDrainingBehaviour`, `FluidFillingBehaviour`, `HosePulleyRenderer`): a copper reel that
//! lets a hose down by rotation and either drains the body of fluid at the hose's end (breadth
//! first through connected sources, a bucket at a time, from the top) or fills the space there
//! with a bucket of what the pipes push in. A body of 10,000 blocks or more of water or lava
//! counts as infinite: it is never drained and swallows fills. Every decision, the searches and the
//! tank are `create_kinetics::rules::hose_pulley`; this file reads the world, calls them and
//! applies what they ask.
//!
//! | Block | Behaviour |
//! |---|---|
//! | `create:hose_pulley` | Shaft on one side (4 su per RPM), a pipe-facing fluid handler of 1,500 mB on the other that drains or fills the world at the hose's end, hose drawn as the coil, hose and end piece |
//!
//! It joins the fluid network as a handler (`custom_fluid_tanks`, `_fill`, `_drain`), as the tank
//! does: a pump pushes into it or pulls from it through a pipe. Honey and chocolate are mod fluids
//! Horizon draws as water; they are placed and drained as the liquid blocks `create:honey` and
//! `create:chocolate`, as water is.
//!
//! Known differences from Create: see `rules/hose_pulley.rs`. Also, a hose placed beside a shaft or
//! a pipe doesn't turn to meet it (`getPreferredHorizontalFacing` reads neighbours a placement hook
//! can't; it faces away from the player); the pulley's pipe side answers for any side asked of a
//! pump's probe, so a pipe beside its other faces connects; the hose offset is saved as a float
//! and a time, not Create's `LerpedFloat`; the coil's texture doesn't scroll.

use std::collections::HashMap;
use std::sync::Mutex;

use create_kinetics::rules::fluids as fluid_rules;
use create_kinetics::rules::hose_pulley::{
    self as rules, Cell, Change, Config, Drainer, Edit, Filler, Level, Motion, Tank,
};
use create_kinetics::rules::pulley as pulley_rules;
use create_kinetics::rules::saw::horizontal_direction;
use create_kinetics::{Axis, Dir, Kind, Node, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, CustomBlockFluidsEvent,
    CustomFluidDrainEvent, CustomFluidFillEvent, CustomFluidTanksEvent, Flow, Hooks, Host, LevelHost,
    ModBlockEntity, NativeFluidAction, NativeFluidDrain, NativeFluidStack, NativeFluidTankView,
    StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use crate::{kinetic, util};

const ID: &str = rules::HOSE_PULLEY;
/// `AllPartialModels.HOSE_COIL`, `HOSE`, `HOSE_MAGNET`, `HOSE_HALF`, `HOSE_HALF_MAGNET`.
const COIL: &str = "create:block/hose_pulley/hose_coil";
const HOSE: &str = "create:block/hose_pulley/rope";
const MAGNET: &str = "create:block/hose_pulley/pulley_magnet";
const HOSE_HALF: &str = "create:block/hose_pulley/rope_half";
const MAGNET_HALF: &str = "create:block/hose_pulley/rope_half_magnet";
const MODELS: [&str; 5] = [COIL, HOSE, MAGNET, HOSE_HALF, MAGNET_HALF];

// Saved: the hose's snapshot (`Offset`, as `HosePulleyBlockEntity.write`, here a float at
// `SyncTime`), `isMoving`, the internal tank (`Tank`: `id`, `amount`) and `Infinite`.
const OFFSET: &str = "Offset";
const SYNC_TIME: &str = "SyncTime";
const MOVING: &str = "Moving";
const TANK: &str = "Tank";
const INFINITE: &str = "Infinite";

/// The block sounds' category.
const BLOCKS: usize = 4;
/// `BlockEntity.lazyTick`'s rate.
const LAZY: i64 = 10;
/// `Block.UPDATE_CLIENTS | UPDATE_KNOWN_SHAPE`, the flags the behaviours set blocks with.
const FLAGS: i32 = 2 | 16;

struct Hose;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Hose)
}

// --- what a running hose works from between ticks ----------------------------------------------------

/// The behaviours of one pulley. Not saved (`rules/hose_pulley.rs`): a pulley starts searching on its
/// first tick.
struct Behaviours {
    drainer: Drainer,
    filler: Filler,
    seen: i64,
}

type Key = (String, Pos);

static STATES: Mutex<Option<HashMap<Key, Behaviours>>> = Mutex::new(None);

fn with_states<R>(f: impl FnOnce(&mut HashMap<Key, Behaviours>) -> R) -> R {
    let mut guard = STATES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

/// Takes the pulley's behaviours out of the table (never hold the table across a call into the
/// game: placing a block runs hooks again); put them back with [`give`].
fn take(dimension: &str, pos: Pos, cfg: &Config) -> Behaviours {
    with_states(|states| states.remove(&(dimension.to_owned(), pos))).unwrap_or_else(|| Behaviours {
        drainer: Drainer::new(pos, cfg),
        filler: Filler::new(cfg),
        seen: 0,
    })
}

fn give(dimension: &str, pos: Pos, behaviours: Behaviours) {
    with_states(|states| {
        states.insert((dimension.to_owned(), pos), behaviours);
    });
}

// --- the world as the rules see it ----------------------------------------------------------------------

/// The fluid a block of this name is the liquid block of.
fn liquid(name: &str) -> bool {
    matches!(name, "minecraft:water" | "minecraft:lava" | "create:honey" | "create:chocolate")
}

/// Blocks that hold water as their fluid state and have no collision of their own.
fn wet(name: &str) -> bool {
    matches!(
        name,
        "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass" | "minecraft:bubble_column"
    )
}

/// `FluidFillingBehaviour.canBeReplacedByFluid` (`FlowingFluidBlock#isBlocked`): not doors, signs,
/// ladders, sugar cane, bubble columns, portals or structure voids, and nothing that blocks motion
/// (read here as having a collision box).
fn replaceable_by_fluid(host: &dyn Host, pos: Pos, name: &str) -> bool {
    let path = name.rsplit(':').next().unwrap_or(name);
    if path.ends_with("_door")
        || path.ends_with("_sign")
        || path.ends_with("_wall_sign")
        || path.ends_with("_hanging_sign")
        || matches!(
            path,
            "ladder" | "sugar_cane" | "bubble_column" | "nether_portal" | "end_portal" | "end_gateway" | "structure_void"
        )
    {
        return false;
    }
    host.block_collision(pos).is_empty()
}

/// A read-only view of the level.
struct World<'a> {
    host: &'a dyn Host,
    dimension: &'a str,
}

impl Level for World<'_> {
    fn loaded(&self, pos: Pos) -> bool {
        self.host.chunk_is_loaded(self.dimension, [pos[0] >> 4, pos[2] >> 4])
    }

    fn cell(&self, pos: Pos) -> Cell {
        let name = self.host.block(pos);
        if liquid(&name) {
            let level = self
                .host
                .block_property(pos, "level")
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(0);
            return Cell::Liquid { fluid: name, level };
        }
        if wet(&name) {
            return Cell::Wet { fluid: "minecraft:water".to_owned() };
        }
        if let Some(logged) = self.host.block_property(pos, "waterlogged") {
            return Cell::Waterlogged(logged == "true");
        }
        let replaceable_by_fluid = replaceable_by_fluid(self.host, pos, &name);
        Cell::Other { replaceable_by_fluid }
    }

    fn ultra_warm(&self) -> bool {
        self.dimension == "minecraft:the_nether"
    }
}

/// The level the behaviours may change.
struct Editor<'a> {
    host: &'a mut dyn Host,
    dimension: &'static str,
}

impl Level for Editor<'_> {
    fn loaded(&self, pos: Pos) -> bool {
        World { host: &*self.host, dimension: self.dimension }.loaded(pos)
    }

    fn cell(&self, pos: Pos) -> Cell {
        World { host: &*self.host, dimension: self.dimension }.cell(pos)
    }

    fn ultra_warm(&self) -> bool {
        self.dimension == "minecraft:the_nether"
    }
}

impl Edit for Editor<'_> {
    fn set(&mut self, pos: Pos, change: Change) {
        match change {
            Change::Air => {
                if let Some(air) = self.host.default_state("minecraft:air") {
                    self.host.set_block(self.dimension, pos, air, FLAGS);
                }
            }
            Change::Unwaterlog => {
                self.host.set_block_property(pos, "waterlogged", "false");
            }
            Change::Waterlog => {
                self.host.set_block_property(pos, "waterlogged", "true");
                // `updatePostWaterlogging`: a lit block is put out.
                if self.host.block_property(pos, "lit").is_some() {
                    self.host.set_block_property(pos, "lit", "false");
                }
            }
            Change::Source(fluid) => {
                // `replaceBlock`: what stood there drops what it drops.
                let existing = self.host.block(pos);
                if existing != "minecraft:air" && !liquid(&existing) && !wet(&existing) {
                    self.host.destroy_block(pos, None, true);
                }
                if let Some(state) = self.host.default_state(&fluid) {
                    self.host.set_block(self.dimension, pos, state, FLAGS);
                }
            }
        }
    }

    fn effect(&mut self, pos: Pos, fluid: &str, fill_sound: bool) {
        let sound = if fill_sound { fluid_rules::fill_sound(fluid) } else { fluid_rules::empty_sound(fluid) };
        self.host.play_sound(sound, util::centre(pos), BLOCKS, 0.3, 1.0);
    }

    fn evaporates(&mut self, pos: Pos) {
        self.host.play_sound("minecraft:block.fire.extinguish", util::centre(pos), BLOCKS, 0.5, 2.6);
    }
}

// --- the saved state -------------------------------------------------------------------------------------

fn flag(value: bool) -> Tag {
    Tag::Byte(i8::from(value))
}

fn put(entity: &mut ModBlockEntity, key: &str, value: Tag) -> bool {
    if util::differs(&entity.nbt, key, &value) {
        entity.nbt.insert(key.into(), value);
        true
    } else {
        false
    }
}

fn motion_of(nbt: &Compound) -> Motion {
    Motion {
        offset: util::float(nbt, OFFSET).unwrap_or(0.0),
        time: util::long(nbt, SYNC_TIME),
        // A fresh hose is moving (`isMoving = true`).
        moving: !util::has(nbt, MOVING) || util::boolean(nbt, MOVING),
    }
}

fn write_motion(entity: &mut ModBlockEntity, motion: &Motion) -> bool {
    let mut changed = false;
    if util::float(&entity.nbt, OFFSET) != Some(motion.offset) {
        util::set_float(&mut entity.nbt, OFFSET, motion.offset);
        changed = true;
    }
    changed |= put(entity, SYNC_TIME, Tag::Long(motion.time));
    changed |= put(entity, MOVING, flag(motion.moving));
    changed
}

fn tank_of(nbt: &Compound) -> Tank {
    let Some(Tag::Compound(tank)) = util::get(nbt, TANK) else {
        return Tank::default();
    };
    let fluid = match util::get(tank, "id") {
        Some(Tag::String(id)) => id.to_string_lossy(),
        _ => String::new(),
    };
    let amount = util::int(tank, "amount");
    if fluid.is_empty() || fluid == "minecraft:empty" || amount <= 0 {
        Tank::default()
    } else {
        Tank { fluid: Some(fluid), amount }
    }
}

fn tank_tag(tank: &Tank) -> Tag {
    let mut tag = Compound::new();
    tag.insert("id".into(), Tag::String(tank.fluid.as_deref().unwrap_or("minecraft:empty").into()));
    tag.insert("amount".into(), Tag::Int(if tank.is_empty() { 0 } else { tank.amount }));
    Tag::Compound(tag)
}

fn facing_of(host: &dyn Host, pos: Pos) -> Option<Dir> {
    util::facing(host, pos, "facing")
}

/// `HosePulleyBlock.getRotationAxis` and `hasShaftTowards`: the shaft side, one turn clockwise of
/// the facing.
fn shaft_face(facing: Dir) -> Dir {
    facing.clockwise_about(Axis::Y)
}

/// `HosePulleyBlock.hasPipeTowards`: the side a pipe connects to.
fn pipe_face(facing: Dir) -> Dir {
    shaft_face(facing).opposite()
}

fn cfg() -> Config {
    Config::default()
}

/// Whether the block `depth` below `pos` `canBeReplaced`.
fn replaceable(host: &dyn Host, pos: Pos) -> impl Fn(i32) -> bool + '_ {
    move |depth| {
        let at = rules::below(pos, depth);
        let state = create_kinetics::rules::contraption::State::new(&host.block(at));
        create_kinetics::rules::contraption::replaceable(&state) || liquid(&state.name)
    }
}

/// Where the hose hangs now: the block its end is in (`worldPosition.below(ceil(offset))`).
fn root_of(host: &dyn Host, pos: Pos, entity: &ModBlockEntity) -> (Pos, Motion) {
    let motion = motion_of(&entity.nbt);
    let now = util::game_time(host) as f64;
    let offset = motion.offset_at(kinetic::speed(entity), now);
    (rules::below(pos, rules::end_depth(offset)), motion)
}

// --- the fluid handler -----------------------------------------------------------------------------------

/// Whether the side asked about is the pipe's.
fn pipe_side(host: &dyn Host, pos: Pos, side: Option<crate::compat::Direction>) -> bool {
    let Some(side) = side else { return true };
    facing_of(host, pos).is_some_and(|facing| util::dir(side) == pipe_face(facing))
}

impl Hose {
    fn views(&self, host: &dyn Host, pos: Pos) -> Vec<NativeFluidTankView> {
        let Some(entity) = host.mod_block_entity(pos) else {
            return Vec::new();
        };
        let tank = tank_of(&entity.nbt);
        let (root, _) = root_of(host, pos, &entity);
        let dimension = host.dimension();
        let cfg = cfg();
        let mut behaviours = take(dimension, pos, &cfg);
        let drainable = if tank.is_empty() {
            behaviours.drainer.drainable(&World { host, dimension }, &cfg, root)
        } else {
            None
        };
        give(dimension, pos, behaviours);
        let held = rules::shown(&tank, None);
        let mut views = vec![NativeFluidTankView {
            tank: 0,
            content: held.map(|(fluid, amount)| NativeFluidStack { fluid, amount, components: Compound::new() }),
            capacity: rules::TANK_CAPACITY,
            can_fill: true,
            can_drain: true,
        }];
        // `getFluidInTank`: with nothing in the tank, a bucket of what could be drained.
        if let Some(fluid) = drainable {
            views.push(NativeFluidTankView {
                tank: 1,
                content: Some(NativeFluidStack { fluid, amount: rules::BUCKET, components: Compound::new() }),
                capacity: rules::BUCKET,
                can_fill: false,
                can_drain: true,
            });
        }
        views
    }
}

fn store_tank(host: &mut dyn Host, pos: Pos, tank: &Tank) {
    let tag = tank_tag(tank);
    host.change_mod_block_entity(pos, &mut |entity| {
        put(entity, TANK, tag.clone());
    });
}

impl Hooks for Hose {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in MODELS {
            event.models.push(model.to_owned());
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        if event.block != ID {
            return None;
        }
        let mut nbt = kinetic::new_entity();
        util::set_float(&mut nbt, OFFSET, 0.0);
        nbt.insert(SYNC_TIME.into(), Tag::Long(0));
        nbt.insert(MOVING.into(), flag(true));
        nbt.insert(INFINITE.into(), flag(false));
        Some((ID.to_owned(), nbt))
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
        let dimension = host.dimension();
        let Some(facing) = facing_of(&*host, pos) else {
            return;
        };
        let node = Node {
            impact: rules::IMPACT,
            ..Node::passive(Kind::shafts(&[shaft_face(facing)]))
        };
        let previous = kinetic::speed(event.entity);
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node) else {
            // The network jammed and the pulley broke.
            with_states(|states| states.remove(&(dimension.to_owned(), pos)));
            return;
        };
        let now = util::game_time(&*host);
        let cfg = cfg();
        let mut sync = changed;
        let mut motion = motion_of(&event.entity.nbt);
        let mut behaviours = take(dimension, pos, &cfg);
        behaviours.seen = now;

        // `onSpeedChanged`: follows the old speed up to now, then rounds a stopped hose or starts
        // a moving one's searches afresh.
        if changed {
            let (offset, moving, reset) = {
                let free = replaceable(&*host, pos);
                let current = motion.offset_at(previous, now as f64);
                rules::speed_changed(current, rpm, &free)
            };
            motion = Motion { offset, time: now, moving };
            if reset {
                behaviours.drainer.reset(&cfg);
                behaviours.filler.reset(&cfg);
            }
            sync |= write_motion(event.entity, &motion);
        }

        // `tick`: a moving hose lowers (or raises) by one step; it stops against a block, at the
        // pulley or when the speed is gone. `lazyTick`: a stopped one starts again when the way
        // below is free.
        {
            let free = replaceable(&*host, pos);
            let speed = rules::movement_speed(rpm);
            if motion.moving {
                let current = motion.offset_at(rpm, now as f64);
                let (next, moving) = rules::tick(current, true, speed, rpm == 0.0, &free);
                if !moving {
                    motion = Motion { offset: next, time: now + 1, moving: false };
                    sync |= write_motion(event.entity, &motion);
                }
            } else if now.rem_euclid(LAZY) == 0 && rules::restarts(motion.offset, rpm, &free) {
                motion = Motion { offset: motion.offset, time: now, moving: true };
                behaviours.drainer.reset(&cfg);
                behaviours.filler.reset(&cfg);
                sync |= write_motion(event.entity, &motion);
            }
        }

        // The behaviours' own ticks (searching, validating).
        {
            let world = World { host: &*host, dimension };
            behaviours.drainer.tick(&world, &cfg);
            behaviours.filler.tick(&world, &cfg);
        }
        let infinite = behaviours.drainer.infinite() || behaviours.filler.infinite();
        sync |= put(event.entity, INFINITE, flag(infinite));
        behaviours.drainer.dirty = false;
        behaviours.filler.dirty = false;
        give(dimension, pos, behaviours);
        if now.rem_euclid(200) == 0 {
            with_states(|states| states.retain(|_, b| now - b.seen < 400));
        }
        if sync {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        let entity = event.entity;
        if entity.id != ID {
            return None;
        }
        let motion = motion_of(&entity.nbt);
        let mut tag = kinetic::update_tag(entity);
        util::set_float(&mut tag, OFFSET, motion.offset);
        tag.insert(SYNC_TIME.into(), Tag::Long(motion.time));
        tag.insert(MOVING.into(), flag(motion.moving));
        tag.insert(INFINITE.into(), flag(util::boolean(&entity.nbt, INFINITE)));
        Some(tag)
    }

    /// `HosePulleyRenderer`: the shaft, the coil, and the end piece and hose below it (the drawn
    /// offset never above 3/16).
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        let axis = shaft_face(facing).axis();
        let draw = &mut *event.draw;
        kinetic::draw_shaft(host, draw, axis, rpm);
        // The coil is authored along X, like the block: it turns with the block's own rotation.
        draw.push();
        let turn = match facing {
            Dir::East => 90.0,
            Dir::South => 180.0,
            Dir::West => 270.0,
            _ => 0.0,
        };
        if turn != 0.0 {
            draw.rotate_about([0.0, 1.0, 0.0], turn, [0.5; 3]);
        }
        host.draw_model(draw, COIL, 0xFF_FFFF);
        draw.pop();
        let motion = motion_of(&event.entity.nbt);
        let now = draw.game_time as f64 + f64::from(draw.partial);
        let offset = rules::drawn_offset(motion.offset_at(rpm, now));
        // `AbstractPulleyRenderer.renderSafe` with `isRunning` always true.
        let plan = pulley_rules::plan(offset, true);
        if let Some((y, half)) = plan.magnet {
            draw.push();
            draw.translate([0.0, -y, 0.0]);
            host.draw_model(draw, if half { MAGNET_HALF } else { MAGNET }, 0xFF_FFFF);
            draw.pop();
        }
        if let Some(y) = plan.half_rope {
            draw.push();
            draw.translate([0.0, -y, 0.0]);
            host.draw_model(draw, HOSE_HALF, 0xFF_FFFF);
            draw.pop();
        }
        for y in &plan.ropes {
            draw.push();
            draw.translate([0.0, -y, 0.0]);
            host.draw_model(draw, HOSE, 0xFF_FFFF);
            draw.pop();
        }
        // Only a hose at rest is the same each frame.
        if !motion.moving {
            draw.keep();
        }
    }

    /// `createRenderBoundingBox`: the hose hangs below the pulley.
    fn block_entity_render_bounding_box(&self, host: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != ID {
            return;
        }
        let motion = motion_of(&event.entity.nbt);
        let now = util::game_time(host) as f64;
        let offset = motion.offset_at(kinetic::speed(event.entity), now);
        let reach = rules::end_depth(offset).max(rules::end_depth(motion.offset)) as f32 + 2.0;
        event.bounds[1] = event.bounds[1].min(-reach);
    }

    /// `HosePulleyBlock.getStateForPlacement` without the shaft or pipe beside it it would prefer:
    /// facing away from the player.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if &*event.context.item != ID {
            return None;
        }
        let facing = horizontal_direction(event.context.yaw).opposite();
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// The Wrench (`IWrenchable.getRotatedBlockState`): a click on the top or bottom turns the
    /// block a quarter clockwise; sneaking picks it up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let pos = event.pos;
        let clicked = Dir::from_normal(event.face);
        util::wrench(host, pos, |host| {
            if !matches!(clicked, Some(Dir::Up | Dir::Down)) {
                return;
            }
            if let Some(facing) = facing_of(&*host, pos) {
                host.set_block_property(pos, "facing", facing.clockwise_about(Axis::Y).name());
            }
        })
    }

    // --- the fluid handler: `registerCapabilities` (the pipe side, or no side) ---

    fn custom_block_fluids(&self, _: &dyn Host, event: &mut CustomBlockFluidsEvent<'_>) {
        if event.block == ID {
            event.custom = true;
        }
    }

    fn custom_fluid_tanks(&self, host: &dyn Host, event: &mut CustomFluidTanksEvent) -> Option<Vec<NativeFluidTankView>> {
        if host.block(event.pos) != ID {
            return None;
        }
        // Other sides have a handler too but reach nothing (a pump's probe asks for any side).
        if !pipe_side(host, event.pos, event.side) {
            return Some(Vec::new());
        }
        Some(self.views(host, event.pos))
    }

    /// `HosePulleyFluidHandler.fill`.
    fn custom_fluid_fill(&self, host: &mut dyn Host, event: &mut CustomFluidFillEvent<'_>) -> Option<i32> {
        let pos = event.pos;
        if host.block(pos) != ID {
            return None;
        }
        if !pipe_side(&*host, pos, event.side) {
            return Some(0);
        }
        let resource = event.resource;
        if !resource.components.is_empty() {
            return Some(0);
        }
        let entity = host.mod_block_entity(pos)?;
        let (root, motion) = root_of(&*host, pos, &entity);
        let dimension = host.dimension();
        let cfg = cfg();
        let mut tank = tank_of(&entity.nbt);
        let before = tank.clone();
        let mut behaviours = take(dimension, pos, &cfg);
        let accepted = {
            let mut editor = Editor { host: &mut *host, dimension };
            rules::handler_fill(
                &mut tank,
                &mut behaviours.drainer,
                &mut behaviours.filler,
                &mut editor,
                &cfg,
                root,
                motion.moving,
                &resource.fluid,
                resource.amount,
                event.action == NativeFluidAction::Execute,
            )
        };
        give(dimension, pos, behaviours);
        if tank != before {
            store_tank(host, pos, &tank);
        }
        Some(accepted)
    }

    /// `HosePulleyFluidHandler.drain`.
    fn custom_fluid_drain(&self, host: &mut dyn Host, event: &mut CustomFluidDrainEvent<'_>) -> Option<NativeFluidStack> {
        let pos = event.pos;
        if host.block(pos) != ID {
            return None;
        }
        if !pipe_side(&*host, pos, event.side) {
            return None;
        }
        let (max, only) = match event.request {
            NativeFluidDrain::Any(max) => (*max, None),
            NativeFluidDrain::Exact(stack) => {
                if !stack.components.is_empty() {
                    return None;
                }
                (stack.amount, Some(stack.fluid.clone()))
            }
            _ => return None,
        };
        let entity = host.mod_block_entity(pos)?;
        let (root, motion) = root_of(&*host, pos, &entity);
        let dimension = host.dimension();
        let cfg = cfg();
        let mut tank = tank_of(&entity.nbt);
        let before = tank.clone();
        let mut behaviours = take(dimension, pos, &cfg);
        let given = {
            let mut editor = Editor { host: &mut *host, dimension };
            rules::handler_drain(
                &mut tank,
                &mut behaviours.drainer,
                &mut behaviours.filler,
                &mut editor,
                &cfg,
                root,
                motion.moving,
                max,
                only.as_deref(),
                event.action == NativeFluidAction::Execute,
            )
        };
        give(dimension, pos, behaviours);
        if tank != before {
            store_tank(host, pos, &tank);
        }
        given.map(|(fluid, amount)| NativeFluidStack { fluid, amount, components: Compound::new() })
    }
}
