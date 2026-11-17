//! The Mechanical Drill (`DrillBlock`, `DrillBlockEntity`, `DrillRenderer`) and the block breaking
//! it shares with the Mechanical Saw facing sideways (`BlockBreakingKineticBlockEntity`): a shaft
//! out of the back, and the block in front broken a stage at a time, the crack shown, the drops
//! left in the world. The rules (hardness, the break timer, damage) are
//! `create_kinetics::rules::drill`; this is the glue.
//!
//! Known differences from Create:
//! - Stationary drills only: the drill on a contraption (`DrillMovementBehaviour`) is a later slice.
//! - The cobblestone generator optimisation (`DrillBlockEntity.optimiseCobbleGen`) is left out:
//!   the drops always land in the world, never straight into a belt, hopper or chute.
//! - Horizon's `Host` cannot ask a block its hardness (`getDestroySpeed`), so it comes from a table
//!   built from the game's data (vanilla blocks) and this mod's declarations; another mod's block
//!   is taken as hardness 2.0.
//! - A block tag that does not exist reads as empty: `create:non_breakable` is empty in Create.
//! - No hit sound at each stage (`SoundType.getHitSound`): the host cannot name a block's sound.
//! - The head hurts what stands in it from a scan of the entities near it every 4 ticks
//!   (`Block.entityInside` is heard by no hook with a `Host`), at most as often as an entity's
//!   invulnerability allows anyway.
//! - A drill is not told when a neighbour changes (`neighborChanged` -> `destroyNextTick`): a
//!   wait between two crack stages runs out first. The crack of a drill that is broken or
//!   unloaded mid-way fades by itself after 20 seconds.
//! - A placed drill does not line up with a shaft beside it (`getPreferredFacing`), and keeps no
//!   placement helper (`PlacementHelpers`: right-clicking a drill with a drill).

use create_kinetics::rules::drill::{
    self as rules, Breaker, DAMAGE_SCAN, DROP_SPREAD, PICKUP_DELAY, Step, Target,
};
use create_kinetics::{Dir, Kind, Node, Pos, looking};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost, ModBlockEntity,
    RandomSource, StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use crate::{kinetic, util};

const ID: &str = "create:mechanical_drill";
/// `AllPartialModels.DRILL_HEAD`.
pub(super) const HEAD: &str = "create:block/mechanical_drill/head";
const CENTRE: [f32; 3] = [0.5; 3];
/// `CreateDamageSources.drill`.
const DAMAGE_TYPE: &str = "create:mechanical_drill";

// Saved as `BlockBreakingKineticBlockEntity.write` names them.
const PROGRESS: &str = "Progress";
const NEXT_TICK: &str = "NextTick";
const BREAKING: &str = "Breaking";

struct Drill;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Drill)
}

// --- what the drill and the saw share (`BlockBreakingKineticBlockEntity`) -------------------------

/// What a breaker does besides the common part: the saw only breaks what it can saw
/// (`SawBlockEntity.canBreak`) and fells the tree it cuts (`onBlockBroken`).
pub(super) trait Tool {
    /// `canBreak` beyond `isBreakable`: the saw's `isSawable`.
    fn allowed(&self, _host: &dyn Host, _at: Pos, _block: &str) -> bool {
        true
    }
    /// `onBlockBroken`: breaks the block at `breaking` (a drill's drop) and whatever follows.
    fn broken(&self, host: &mut dyn Host, _pos: Pos, breaking: Pos, _block: &str) {
        drop_block(host, breaking);
    }
}

/// A drill's `onBlockBroken` and `BlockHelper.destroyBlock`: the block breaks, its drops are made
/// where it stood (within an eighth of a block of its centre), at rest.
pub(super) fn drop_block(host: &mut dyn Host, breaking: Pos) {
    let dimension = host.dimension();
    let centre = util::centre(breaking);
    // `VecHelper.offsetRandomly`: each axis moved by (random - .5) * 2 * radius.
    let mut at = centre;
    if let Some(random) = host.level_random(dimension) {
        for value in &mut at {
            *value += f64::from((random.next_float() - 0.5) * 2.0 * DROP_SPREAD);
        }
    }
    let Some(drops) = host.destroy_block(breaking, None, true) else {
        return;
    };
    for stack in drops {
        util::spawn_item(host, at, &stack, [0.0; 3], PICKUP_DELAY);
    }
}

fn target_of(host: &dyn Host, at: Pos, tool: &dyn Tool) -> (Target, String) {
    let block = host.block(at);
    let hardness = rules::hardness(&block);
    // A tag nobody declares is empty (`create:non_breakable` is, in Create).
    let non_breakable = host
        .block_name_in_tag(&block, "create:non_breakable")
        .unwrap_or(false);
    let breakable = rules::is_breakable(&block, hardness, non_breakable) && tool.allowed(host, at, &block);
    (
        Target {
            hardness,
            breakable,
        },
        block,
    )
}

pub(super) fn breaker_of(entity: &ModBlockEntity) -> Breaker {
    Breaker {
        progress: util::int(&entity.nbt, PROGRESS),
        next: util::int(&entity.nbt, NEXT_TICK),
    }
}

pub(super) fn new_breaker_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    nbt.insert(PROGRESS.into(), Tag::Int(0));
    nbt.insert(NEXT_TICK.into(), Tag::Int(0));
    nbt
}

/// Writes the breaker's state, but only what differs: reaching a block entity's fields mutably
/// counts as a change.
fn store(entity: &mut ModBlockEntity, before: Breaker, after: Breaker) {
    if after.progress != before.progress {
        entity.nbt.insert(PROGRESS.into(), Tag::Int(after.progress));
    }
    if after.next != before.next {
        entity.nbt.insert(NEXT_TICK.into(), Tag::Int(after.next));
    }
}

/// One tick of `BlockBreakingKineticBlockEntity.tick` after the network's part, for a breaker at
/// `pos` facing `facing` that `shouldRun`, turning at `rpm`. `nudge` is the saw's
/// `if (shouldRun() && ticksUntilNextProgress < 0) destroyNextTick()`.
pub(super) fn run_breaker(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    rpm: f32,
    facing: Dir,
    runs: bool,
    nudge: bool,
    tool: &dyn Tool,
) {
    let before = breaker_of(entity);
    let mut breaker = before;
    if runs && nudge && breaker.next < 0 {
        breaker.destroy_next_tick();
    }
    // `SmartBlockEntity.tick`'s lazy tick comes first, whatever the breaker does after it.
    breaker.lazy_tick();
    if !runs || rpm == 0.0 {
        store(entity, before, breaker);
        return;
    }
    let breaking = util::add(pos, facing.normal());
    let wanted = Tag::IntArray(breaking.to_vec());
    if util::differs(&entity.nbt, BREAKING, &wanted) {
        entity.nbt.insert(BREAKING.into(), wanted);
    }
    let id = rules::breaker_id(pos);
    let mut seen = String::new();
    let step = {
        let host: &dyn Host = &*host;
        breaker.tick(rpm, || {
            let (target, block) = target_of(host, breaking, tool);
            seen = block;
            target
        })
    };
    // Stored first: breaking a block runs other hooks, and the entity is ours alone here.
    store(entity, before, breaker);
    match step {
        Step::Wait => {}
        Step::Blocked { clear } => {
            if clear {
                host.set_break_progress(id, breaking, -1);
            }
        }
        Step::Cracking { stage } => host.set_break_progress(id, breaking, stage),
        Step::Broken => {
            tool.broken(host, pos, breaking, &seen);
            host.set_break_progress(id, breaking, -1);
        }
    }
}

/// `DrillBlock.entityInside` for the drill at `pos` and the saw: everything but items that
/// stands in the head takes `damage_type` damage for the speed.
pub(super) fn hurt_entities(host: &mut dyn Host, pos: Pos, rpm: f32, damage_type: &str) {
    let dimension = host.dimension();
    let [x, y, z] = pos.map(f64::from);
    // `entities_in` looks at feet: reach for those whose box comes up into the block.
    let (lo, hi) = ([x - 1.0, y - 3.0, z - 1.0], [x + 2.0, y + 1.0, z + 2.0]);
    let damage = rules::damage(rpm);
    for id in host.entities_in(dimension, lo, hi) {
        let Some(entity) = host.entity(id) else {
            continue;
        };
        if host.entity_kind(entity).as_deref() == Some("minecraft:item") {
            continue;
        }
        let Some(bounds) = host.entity_bounding_box(entity) else {
            continue;
        };
        if rules::in_head(bounds, pos) {
            host.hurt_entity(dimension, id, damage, damage_type, None);
        }
    }
}

/// Whether the entity scan runs this tick.
pub(super) fn scan_due(host: &dyn Host, pos: Pos) -> bool {
    // Spread over the ticks, so drills side by side don't all scan together.
    let phase = i64::from(pos[0].wrapping_add(pos[2]).rem_euclid(DAMAGE_SCAN as i32));
    (util::game_time(host) + phase).rem_euclid(DAMAGE_SCAN) == 0
}

/// A shaft out of the back only: `hasShaftTowards(face == facing.getOpposite())`.
pub(super) fn node(facing: Dir) -> Node {
    Node {
        impact: rules::IMPACT,
        ..Node::passive(Kind::shafts(&[facing.opposite()]))
    }
}

/// `CachedBuffers.partialFacing(partial, state)` of a model made facing south: turned to `facing`.
pub(super) fn draw_facing(
    host: &dyn Host,
    draw: &mut crate::compat::BlockEntityDraw,
    model: &str,
    facing: Dir,
) {
    draw.push();
    crate::draw::face_south(draw, facing);
    host.draw_model(draw, model, 0xFF_FFFF);
    draw.pop();
}

// --- hooks --------------------------------------------------------------------------------------

impl Tool for Drill {}

impl Hooks for Drill {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(HEAD.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), new_breaker_entity()))
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
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node(facing)) else {
            return; // The network jammed and the block broke.
        };
        if changed {
            host.block_entity_sync(pos);
        }
        run_breaker(host, pos, event.entity, rpm, facing, true, false, self);
        if rpm != 0.0 && scan_due(host, pos) {
            hurt_entities(host, pos, rpm, DAMAGE_TYPE);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        (event.entity.id == ID).then(|| kinetic::update_tag(event.entity))
    }

    /// `DrillRenderer`: the casing is in the chunk mesh; the head turns about the facing axis.
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
        let draw = &mut *event.draw;
        draw.spin(facing.axis().unit(), CENTRE, rpm * 0.3, 0.0);
        draw_facing(host, draw, HEAD, facing);
        draw.still();
        // Only the steady turn through `spin` is kept: a new speed arrives as a new update tag.
        draw.keep();
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
        let facing = rules::placement_facing(look, context.sneaking);
        level
            .state_with_property(event.state, "facing", facing.name())
            .map(Some)
    }

    /// The Wrench turns the drill round the face clicked (`IWrenchable.getRotatedBlockState`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some(facing), Some(clicked)) = (util::facing(host, pos, "facing"), clicked) else {
                return;
            };
            let next = rules::wrench_facing(facing, clicked);
            if next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
        })
    }
}
