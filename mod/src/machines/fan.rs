//! The Encased Fan in the game (`EncasedFanBlock`, `EncasedFanBlockEntity`, `AirCurrent`,
//! `EncasedFanRenderer`, `FanProcessing`): a shaft out of its back, a current of air in front of
//! it that follows its speed, entities pushed or pulled along it, and items in it processed by
//! what the current passes through. The rules are `create_kinetics::rules::fan`; this is glue.
//!
//! Known differences from Create, for the README:
//! - The current is rebuilt every 30 ticks and when the speed or facing changes, not also when a
//!   neighbour changes. Placing a block in front takes up to 1.5 seconds to cut the air.
//! - Smoking and blasting read copies of vanilla's smoking, smelting and blasting recipes
//!   (`create:baked_smoking`, `create:baked_smelting` and `create:baked_blasting`, made from the
//!   prepared game data by `tools/content/_fan_cooking_gen.py`), because Horizon's `Host` can't
//!   read vanilla's recipes; a data pack's changes to them are not seen. Items that are fire
//!   resistant are the baked item tag `create:fan_fire_resistant` (the `minecraft:damage_resistant`
//!   of the game's items; `FIRE_RESISTANT` in Create 1.21.1). DROP THESE COPIES when the project
//!   moves to the new mod API's `recipe_for`.
//! - Items on depots and belts under the current are not processed (only item entities are).
//! - Players are pushed by a message to their client (`push_player`), which a dedicated server's
//!   movement check may not like; their fall distance and the anti-fly timer are not reset.
//! - Mobs are not blinded or slowed by soul fire (the `Host` adds effects to players only), and a
//!   horse isn't turned into a skeleton horse.
//! - A processing recipe's crafting remainder (a bucket) is not returned.
//! - No particles, no wind sound, no Encased Fan advancements, no chute interplay, and a placed
//!   fan doesn't line up with a shaft beside it (`getPreferredFacing`).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::fan::{
    self, Aabb, Cell, Flame, Heat, Kind, Known, Passage, Progress, Segment,
};
use create_kinetics::{Dir, Kind as NodeKind, Node, Pos, looking};
use crate::compat::{
    AttachmentCodec, AttachmentError, AttachmentType, BlockEntityEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound,
    EntityRef, Flow, Hooks, Holder, Host, ItemStack, LevelHost, NativeEffect, NativeServerPlayer,
    NetFromServerEvent, RandomSource, StandaloneModelsEvent, StateForPlacementEvent, Tag,
    UseItemOnEvent,
};

use crate::{kinetic, util};

pub const ID: &str = "create:encased_fan";
const PROPELLER: &str = "create:block/encased_fan/propeller";
const CENTRE: [f32; 3] = [0.5; 3];
/// The channel the server tells a player's client to push them on (`push_player`).
const PUSH_CHANNEL: &str = "create:fan_push";
/// `SoundSource.NEUTRAL`.
const NEUTRAL: usize = 6;

struct Fan;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Fan)
}

// --- item entities remember their processing (`CreateData.Processing`) -------------------------

static PROCESSING: AttachmentType<Progress> = AttachmentType {
    namespace: "create",
    name: "fan_processing",
    default: Progress::default,
    codec: Some(AttachmentCodec {
        encode: |progress| {
            let mut tag = Compound::new();
            if let Some(kind) = &progress.kind {
                tag.insert("Type".into(), Tag::String(kind.as_str().into()));
            }
            tag.insert("Time".into(), Tag::Int(progress.time));
            Ok(tag)
        },
        decode: |tag| {
            let kind = match tag.get(&"Type".into()) {
                Some(Tag::String(kind)) => Some(kind.to_string_lossy()),
                _ => None,
            };
            match tag.get(&"Time".into()) {
                Some(Tag::Int(time)) => Ok(Progress { kind, time: *time }),
                _ => Err(AttachmentError::Decode),
            }
        },
    }),
    copy_on_death: false,
    sync: false,
};

// --- what the server remembers of each running fan ---------------------------------------------

/// `AirCurrent`'s own state, kept per fan in memory: it is rebuilt from the world after a load.
#[derive(Default)]
struct Current {
    facing: Option<Dir>,
    speed: f32,
    built: Option<i64>,
    searched: Option<i64>,
    seen: i64,
    reach: f32,
    pushing: bool,
    segments: Vec<Segment>,
    bounds: Option<([f64; 3], [f64; 3])>,
    caught: Vec<u32>,
}

type Key = (&'static str, Pos);

static CURRENTS: LazyLock<Mutex<HashMap<Key, Current>>> = LazyLock::new(Mutex::default);

/// Never hold the lock across a call into the game.
fn currents() -> std::sync::MutexGuard<'static, HashMap<Key, Current>> {
    CURRENTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn store(key: Key, current: Current, now: i64) {
    let mut map = currents();
    // A fan that was broken or unloaded leaves its entry behind: sweep them now and then.
    if now % 200 == 0 {
        map.retain(|_, c| now - c.seen < 100);
    }
    map.insert(key, current);
}

// --- hooks --------------------------------------------------------------------------------------

impl Hooks for Fan {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(PROPELLER.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), kinetic::new_entity()))
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
        // A shaft out of the back (`EncasedFanBlock.hasShaftTowards`), 2 su per RPM.
        let node = Node {
            impact: fan::IMPACT,
            ..Node::passive(NodeKind::shafts(&[facing.opposite()]))
        };
        let Some((speed, changed)) = kinetic::tick(host, pos, event.entity, node) else {
            return; // The network jammed and the block broke.
        };
        if changed {
            host.block_entity_sync(pos);
        }
        tick_current(host, pos, facing, speed);
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        (event.entity.id == ID).then(|| kinetic::update_tag(event.entity))
    }

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
        draw(host, &mut *event.draw, facing, kinetic::speed(event.entity));
    }

    /// `EncasedFanBlock.getStateForPlacement`, without the chute and the shaft beside it: the
    /// fan faces the player, or away from them when sneaking.
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
        let facing = if context.sneaking { look } else { look.opposite() };
        level
            .state_with_property(event.state, "facing", facing.name())
            .map(Some)
    }

    /// The Wrench turns the fan round the face clicked (`IWrenchable.getRotatedBlockState`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some(facing), Some(clicked)) = (util::facing(host, pos, "facing"), clicked) else {
                return;
            };
            let next = fan::wrench_facing(facing, clicked);
            if next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
        })
    }

    /// The client half of a push: the server sends the change to this player's velocity.
    fn net_from_server(&self, host: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        if event.channel != PUSH_CHANNEL || event.data.len() != 12 {
            return;
        }
        let mut push = [0.0_f32; 3];
        for (i, chunk) in event.data.chunks_exact(4).enumerate() {
            push[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        if push.iter().all(|v| v.is_finite()) {
            host.push_player(push);
        }
    }
}

// --- drawing (`EncasedFanRenderer`) ---------------------------------------------------------------

/// The half shaft out of the back and the propeller in front of it. The propeller turns at
/// `speed * 5` clamped to 80..=1280 either way, times 0.3 degrees a tick.
pub(super) fn draw(
    host: &dyn Host,
    draw: &mut crate::compat::BlockEntityDraw,
    facing: Dir,
    rpm: f32,
) {
    let back = facing.opposite();
    kinetic::draw_shaft_half(host, draw, back, rpm);
    let mut speed = rpm * 5.0;
    if speed > 0.0 {
        speed = speed.clamp(80.0, 64.0 * 20.0);
    }
    if speed < 0.0 {
        speed = speed.clamp(-64.0 * 20.0, -80.0);
    }
    draw.spin(facing.axis().unit(), CENTRE, speed * 0.3, 0.0);
    draw.push();
    // `CachedBuffers.partialFacing(ENCASED_FAN_INNER, state, direction.getOpposite())`: the model
    // is made with its shaft stub towards the front at north, turned like the half shaft.
    crate::draw::face_south(draw, back);
    host.draw_model(draw, PROPELLER, 0xFF_FFFF);
    draw.pop();
    draw.still();
    // Only the steady turn through `spin` is kept: a new speed arrives as a new update tag.
    draw.keep();
}

// --- the air current (`AirCurrent`) ---------------------------------------------------------------

fn offset(pos: Pos, direction: Dir, steps: i32) -> Pos {
    let n = direction.normal();
    [pos[0] + n[0] * steps, pos[1] + n[1] * steps, pos[2] + n[2] * steps]
}

/// One server tick of the fan's current at `pos`.
fn tick_current(host: &mut dyn Host, pos: Pos, facing: Dir, speed: f32) {
    let dimension = host.dimension();
    let key = (dimension, pos);
    let mut current = currents().remove(&key).unwrap_or_default();
    if speed == 0.0 {
        return; // `rebuild` clears everything when the fan stands still.
    }
    let now = util::game_time(host);
    let stale = current
        .built
        .is_none_or(|built| now - built >= i64::from(fan::BLOCK_CHECK_RATE) || now < built);
    if stale || current.facing != Some(facing) || current.speed != speed {
        rebuild(host, dimension, pos, facing, speed, &mut current);
        current.built = Some(now);
        current.facing = Some(facing);
        current.speed = speed;
        current.searched = None;
    }
    if let Some((min, max)) = current.bounds {
        if current
            .searched
            .is_none_or(|at| now - at >= i64::from(fan::ENTITY_SEARCH) || now < at)
        {
            current.searched = Some(now);
            current.caught = find_entities(host, dimension, min, max);
        }
        affect_entities(host, dimension, pos, facing, speed, &mut current);
    } else {
        current.caught.clear();
    }
    current.seen = now;
    store(key, current, now);
}

/// `AirCurrent.rebuild`: how far the air gets, what it passes through, and where it is.
fn rebuild(host: &dyn Host, dimension: &'static str, pos: Pos, facing: Dir, speed: f32, c: &mut Current) {
    let max = fan::max_distance(speed);
    c.pushing = fan::pushing(speed, facing);
    let level = host.level(dimension);
    c.reach = fan::flow_limit(max, facing, |i| {
        probe(host, level, dimension, offset(pos, facing, i + 1))
    });
    c.segments = fan::segments(c.reach, c.pushing, |i| {
        cell_at(host, offset(pos, facing, i)).kind()
    });
    c.bounds = fan::bounds(pos, facing, c.reach);
}

/// What the air meets at `at` (`getFlowLimit`'s block).
fn probe(
    host: &dyn Host,
    level: Option<crate::compat::LevelRef>,
    dimension: &str,
    at: Pos,
) -> Passage {
    if !level.is_some_and(|level| level.is_loaded(host, at)) {
        return Passage::Unloaded;
    }
    let name = host.block(at);
    if host
        .block_name_in_tag(&name, "create:fan_transparent")
        .unwrap_or(false)
    {
        return Passage::Transparent;
    }
    // A hair inside the block, so only its own collision shape is asked for.
    const E: f64 = 1e-4;
    let [x, y, z] = at.map(f64::from);
    let boxes: Vec<Aabb> = host
        .collision_boxes(dimension, [x + E, y + E, z + E, x + 1.0 - E, y + 1.0 - E, z + 1.0 - E])
        .into_iter()
        .map(|b| [b[0] - x, b[1] - y, b[2] - z, b[3] - x, b[4] - y, b[5] - z])
        .collect();
    Passage::Boxes(boxes)
}

/// `FanProcessingType.getAt`'s inputs at `at`.
fn cell_at(host: &dyn Host, at: Pos) -> Cell {
    let name = host.block(at);
    if matches!(
        name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    ) {
        return Cell::default();
    }
    let in_tag = |tag: &str| host.block_name_in_tag(&name, tag).unwrap_or(false);
    let property = |key: &str| host.block_property(at, key);
    Cell {
        water: fan::holds_water(&name, property("waterlogged").as_deref() == Some("true")),
        lava: name == "minecraft:lava",
        in_splashing: in_tag(Kind::Splashing.catalyst_tag()),
        in_haunting: in_tag(Kind::Haunting.catalyst_tag()),
        in_smoking: in_tag(Kind::Smoking.catalyst_tag()),
        in_blasting: in_tag(Kind::Blasting.catalyst_tag()),
        campfire: in_tag("minecraft:campfires"),
        lit: property("lit").map(|v| v == "true"),
        heat: property("blaze").and_then(|v| Heat::from_name(&v)),
        flame: property("flame_type").and_then(|v| Flame::from_name(&v)),
    }
}

// --- entities in the current --------------------------------------------------------------------

/// A box `[min, max]` and another meet (`AABB.intersects`).
pub(super) fn meets(a: [[f64; 3]; 2], min: [f64; 3], max: [f64; 3]) -> bool {
    (0..3).all(|i| a[0][i] < max[i] && a[1][i] > min[i])
}

/// What the current needs to know of an entity.
pub(super) struct Body {
    pub id: u32,
    pub entity: EntityRef,
    pub kind: String,
    pub position: [f64; 3],
    pub bounds: Option<[[f64; 3]; 2]>,
    pub velocity: [f64; 3],
    pub sneaking: bool,
    /// The player, when it is one.
    pub player: Option<NativeServerPlayer>,
}

impl Body {
    /// `AirCurrent.isPlayerCreativeFlying`.
    pub fn creative_flying(&self) -> bool {
        self.player
            .as_ref()
            .is_some_and(|p| p.game_mode == util::CREATIVE && p.flying)
    }
}

pub(super) fn body(host: &mut dyn Host, id: u32) -> Option<Body> {
    let entity = host.entity(id)?;
    let kind = host.entity_kind(entity)?;
    let position = host.entity_position(entity)?;
    let player = if kind == "minecraft:player" {
        Some(
            host.server_players()
                .into_iter()
                .find(|p| p.entity_id as u32 == id)?,
        )
    } else {
        None
    };
    let sneaking = match &player {
        Some(p) => p.sneaking,
        None => host.entity_is_shift_key_down(entity).unwrap_or(false),
    };
    Some(Body {
        id,
        entity,
        kind,
        position,
        bounds: host.entity_bounding_box(entity),
        velocity: host.entity_velocity(entity).unwrap_or([0.0; 3]),
        sneaking,
        player,
    })
}

/// Moves the body to `velocity`: set for an entity the server moves, a message to a player's
/// client for the change (the player's own game moves them).
pub(super) fn set_velocity(host: &mut dyn Host, body: &Body, velocity: [f64; 3]) {
    match &body.player {
        Some(player) => {
            let mut data = Vec::with_capacity(12);
            for axis in 0..3 {
                data.extend_from_slice(&((velocity[axis] - body.velocity[axis]) as f32).to_le_bytes());
            }
            let _ = host.net_send_to_player(player.uuid, PUSH_CHANNEL, &data);
        }
        None => {
            host.entity_set_velocity(body.entity, velocity);
        }
    }
}

/// `AirCurrent.findEntities`: every item, mob and player whose box meets the current's.
fn find_entities(host: &mut dyn Host, dimension: &str, min: [f64; 3], max: [f64; 3]) -> Vec<u32> {
    let mut ids: Vec<u32> = host
        .item_entities(dimension, min, max)
        .into_iter()
        .map(|item| item.id)
        .collect();
    // `entities_in` looks at feet: reach for those whose box comes up into the current.
    let lo = [min[0] - 1.5, min[1] - 3.0, min[2] - 1.5];
    let hi = [max[0] + 1.5, max[1], max[2] + 1.5];
    for id in host.entities_in(dimension, lo, hi) {
        if ids.contains(&id) {
            continue;
        }
        let Some(entity) = host.entity(id) else {
            continue;
        };
        if host.entity_kind(entity).as_deref() == Some("minecraft:item") {
            continue;
        }
        let touching = match host.entity_bounding_box(entity) {
            Some(bounds) => meets(bounds, min, max),
            None => host.entity_position(entity).is_some_and(|p| {
                (0..3).all(|i| min[i] <= p[i] && p[i] <= max[i])
            }),
        };
        if touching {
            ids.push(id);
        }
    }
    ids
}

/// `AirCurrent.tickAffectedEntities`.
fn affect_entities(
    host: &mut dyn Host,
    dimension: &'static str,
    pos: Pos,
    facing: Dir,
    speed: f32,
    c: &mut Current,
) {
    let Some((min, max)) = c.bounds else {
        return;
    };
    let flow = if c.pushing { facing } else { facing.opposite() };
    let centre = util::centre(pos);
    let mut keep = Vec::with_capacity(c.caught.len());
    for id in std::mem::take(&mut c.caught) {
        let Some(body) = body(host, id) else {
            continue; // Gone (`!entity.isAlive()`).
        };
        let touching = match body.bounds {
            Some(bounds) => meets(bounds, min, max),
            None => (0..3).all(|i| min[i] <= body.position[i] && body.position[i] <= max[i]),
        };
        if !touching || body.creative_flying() {
            continue;
        }
        keep.push(id);

        let from_centre = (0..3)
            .map(|i| (body.position[i] - centre[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        let velocity =
            fan::push_velocity(body.velocity, flow, speed, body.sneaking, from_centre, c.reach);
        set_velocity(host, &body, velocity);

        let distance = fan::aligned_distance_to_face(pos, facing, body.position);
        let Some(kind) = fan::kind_at_offset(&c.segments, c.reach, c.pushing, distance as f32)
        else {
            continue;
        };
        if body.kind == "minecraft:item" {
            process_item(host, dimension, kind, &body, velocity);
        } else {
            affect_entity(host, dimension, kind, &body);
        }
    }
    c.caught = keep;
}

// --- processing items (`FanProcessing`) ------------------------------------------------------------

/// `type.canProcess(stack, level)`. Smoking and blasting read vanilla's cooking recipes, which a
/// mod can't: the copies `create:baked_*` stand in (`rules::fan::BAKED_*`).
fn type_can_process(host: &dyn Host, kind: Kind, stack: &ItemStack) -> bool {
    let baked = |recipe_type: &str| crate::recipes::for_stack(host, recipe_type, stack).is_some();
    match kind {
        Kind::Smoking => baked(fan::BAKED_SMOKING),
        Kind::Blasting => {
            let (smelting, blasting) = (baked(fan::BAKED_SMELTING), baked(fan::BAKED_BLASTING));
            // `stack.has(DataComponents.FIRE_RESISTANT)`, asked only when no recipe takes it.
            let resistant = || host.server_item_in_tag(stack, fan::FIRE_RESISTANT_TAG).unwrap_or(false);
            fan::blasting_can_process(smelting, blasting, !(smelting || blasting) && resistant())
        }
        Kind::Splashing | Kind::Haunting => kind
            .recipe_type()
            .is_some_and(|recipe_type| crate::recipes::for_stack(host, recipe_type, stack).is_some()),
    }
}

/// `RecipeApplier.applyRecipeOn` for a recipe that isn't Create's (a vanilla cooking recipe):
/// `ItemHelper.multipliedOutput(stack, result)`, the result times the count, split by stack size.
fn cooked(host: &dyn Host, stack: &ItemStack, recipe: &crate::recipes::Processing) -> Vec<ItemStack> {
    let Some(out) = recipe.results.first() else {
        return Vec::new();
    };
    let max = host.max_stack_size(&out.stack.item).unwrap_or(64);
    fan::multiplied(stack.count, out.stack.count, max)
        .into_iter()
        .map(|count| ItemStack { count, ..out.stack.clone() })
        .collect()
}

/// `type.process(stack, level)`: `None` where the type does nothing (Create's `null`), an empty
/// list where the item is destroyed.
fn type_process(
    host: &mut dyn Host,
    dimension: &str,
    kind: Kind,
    stack: &ItemStack,
) -> Option<Vec<ItemStack>> {
    match kind {
        // `SmokingType.process`.
        Kind::Smoking => {
            let recipe = crate::recipes::for_stack(host, fan::BAKED_SMOKING, stack)?;
            Some(cooked(host, stack, &recipe))
        }
        // `BlastingType.process`: smelting, else blasting, unless smoking makes the same item.
        Kind::Blasting => {
            let smoking = crate::recipes::for_stack(host, fan::BAKED_SMOKING, stack);
            let smelting = crate::recipes::for_stack(host, fan::BAKED_SMELTING, stack)
                .or_else(|| crate::recipes::for_stack(host, fan::BAKED_BLASTING, stack));
            let first = |recipe: &Option<crate::recipes::Processing>| {
                recipe.as_ref().and_then(|r| r.results.first()).map(|out| out.stack.item.clone())
            };
            let same = || first(&smoking) == first(&smelting);
            match fan::blast(smelting.is_some(), smoking.is_some(), same) {
                fan::Blast::Smelt => smelting.map(|recipe| cooked(host, stack, &recipe)),
                fan::Blast::Destroy => Some(Vec::new()),
            }
        }
        Kind::Splashing | Kind::Haunting => {
            let recipe_type = kind.recipe_type()?;
            let recipe = crate::recipes::for_stack(host, recipe_type, stack)?;
            // `RecipeApplier.applyRecipeOn` (a roll of the results per item, equal stacks merged).
            let mut stacks = Vec::new();
            for _ in 0..stack.count {
                for piece in crate::recipes::roll(host, dimension, &recipe.results) {
                    util::merge(host, &mut stacks, piece);
                }
            }
            Some(stacks)
        }
    }
}

/// `FanProcessing.canProcess` and `applyProcessing` for an item entity, one tick.
fn process_item(host: &mut dyn Host, dimension: &str, kind: Kind, body: &Body, velocity: [f64; 3]) {
    let holder = Holder::Entity(body.id);
    let mut progress = PROCESSING.get_existing(host, holder).ok().flatten();
    let known = fan::can_process(progress.as_ref(), kind);
    if known == Known::No {
        return;
    }
    // The stack itself, from the item entities (`Body` has none).
    let Some(item) = host
        .item_entities(dimension, body.position.map(|v| v - 0.01), body.position.map(|v| v + 0.01))
        .into_iter()
        .find(|item| item.id == body.id)
    else {
        return;
    };
    let stack = item.stack;
    if known == Known::Ask && !type_can_process(host, kind, &stack) {
        // Create asks again every tick; remembering the answer for this type (`Time` -1, as a
        // belt remembers it) saves the recipe lookups.
        let mut remembered = progress.take().unwrap_or_default();
        remembered.kind = Some(kind.id().to_owned());
        remembered.time = -1;
        let _ = PROCESSING.set(host, holder, remembered);
        return;
    }
    let mut state = progress.unwrap_or_default();
    let left = fan::decrement(&mut state, kind, stack.count);
    let _ = PROCESSING.set(host, holder, state);
    if left != 0 {
        return;
    }
    let Some(mut stacks) = type_process(host, dimension, kind, &stack) else {
        return;
    };
    if stacks.is_empty() {
        // `entity.discard()`.
        host.set_item_entity(dimension, body.id, ItemStack::new("minecraft:air", 0));
        return;
    }
    let first = stacks.remove(0);
    host.set_item_entity(dimension, body.id, first);
    for extra in stacks {
        // `entityIn.setDeltaMovement(entity.getDeltaMovement())`.
        util::spawn_item(host, body.position, &extra, velocity, 0);
    }
}

// --- what the current does to other entities (`FanProcessingType.affectEntity`) -------------------

fn affect_entity(host: &mut dyn Host, dimension: &str, kind: Kind, body: &Body) {
    match kind {
        Kind::Splashing => {
            if fan::splashing_hurts(&body.kind) {
                host.hurt_entity(dimension, body.id, 2.0, "minecraft:drown", None);
            }
            if host.entity_fire_ticks(body.entity).unwrap_or(0) > 0 {
                host.entity_set_fire(body.entity, 0);
                let pitch = match host.level_random(dimension) {
                    Some(random) => 1.6 + (random.next_float() - random.next_float()) * 0.4,
                    None => 1.6,
                };
                host.play_sound(
                    "minecraft:entity.generic.extinguish_fire",
                    body.position,
                    NEUTRAL,
                    0.7,
                    pitch,
                );
            }
        }
        Kind::Blasting => burn(host, dimension, body, fan::BLASTING_FIRE_TICKS, fan::BLASTING_DAMAGE, "create:fan_lava"),
        Kind::Smoking => burn(host, dimension, body, fan::SMOKING_FIRE_TICKS, fan::SMOKING_DAMAGE, "create:fan_fire"),
        Kind::Haunting => {
            // Only a player's effects can be changed through the `Host`.
            if let Some(player) = &body.player {
                for (id, (duration, amplifier)) in [
                    ("minecraft:blindness", fan::HAUNTING_BLINDNESS),
                    ("minecraft:slowness", fan::HAUNTING_SLOWNESS),
                ] {
                    host.add_effect(
                        player.uuid,
                        &NativeEffect {
                            id: id.to_owned(),
                            amplifier,
                            duration,
                            ambient: false,
                            particles: false,
                        },
                    );
                }
            }
        }
    }
}

/// `igniteForSeconds` and `hurt` for something that isn't fire immune.
fn burn(host: &mut dyn Host, dimension: &str, body: &Body, ticks: i32, damage: f32, source: &str) {
    if host.entity_fire_immune(body.entity).unwrap_or(false) {
        return;
    }
    // `Entity.igniteForTicks` only ever lengthens the fire.
    if host.entity_fire_ticks(body.entity).unwrap_or(0) < ticks {
        host.entity_set_fire(body.entity, ticks);
    }
    host.hurt_entity(dimension, body.id, damage, source, None);
}
