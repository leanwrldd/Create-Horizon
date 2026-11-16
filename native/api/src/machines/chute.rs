//! The Chute and the Smart Chute (`ChuteBlock`, `SmartChuteBlock`, `AbstractChuteBlock`,
//! `ChuteBlockEntity`, `SmartChuteBlockEntity`, `ChuteItemHandler`, `ChuteRenderer`): items fall
//! down a column of chutes, out of the inventory above into the one below, and an Encased Fan above
//! or below makes them rise. The rules are `create_kinetics::rules::chute`; this is the glue.
//!
//! The item is the block entity's one item slot (`Items`, so hoppers fill and empty it as they do
//! Create's `ChuteItemHandler`, and it drops when the block breaks). Where it is on its way
//! (`ItemPosition`, 0 to 1, as a fraction of the block) and the fans' `Pull` and `Push` are saved
//! with Create's key names; clients are sent them with the game time of the update (`Tick`) and
//! move the item on themselves.
//!
//! Known differences from Create:
//! - Fan pull and push are worked out by each chute from its neighbours every 20 ticks, at once
//!   when a neighbour tells it (a flag on the neighbour's block entity), and every other tick for a
//!   chute with a fan right above or below it. Create updates the whole chain the moment a fan's
//!   speed changes (`EncasedFanBlockEntity.updateChute`), so a change takes a tick or so per
//!   chute to run down the chain here.
//! - A chute's shape, and a diagonal chute turning back to a vertical one, follow its neighbours
//!   every 4 ticks and when it is placed, not on each shape update; a chute placed on the side of
//!   a block faces down again at once where no chute can take its items (`ChuteBlock
//!   .getStateForPlacement` reads the neighbours; Horizon's placement hook can't). The Smart
//!   Chute's `powered` is polled every other tick the same way (`neighborChanged`).
//! - A chute under a diagonal one is not refused at placement (`ChuteBlock.canSurvive`: the
//!   placement hook can't read neighbours).
//! - Putting items into a container is tried for real when the item is half way through the
//!   block, not simulated first and done at the end (see `chute_inv`), so an item enters a
//!   container about two ticks earlier than in Create. An inventory is looked at every other tick
//!   for something to take (Create waits for the inventory to change).
//! - An extraction takes from one slot, or tops up from the next ones, as `ItemHelper.extract` does,
//!   but an inventory that isn't an `Items` list (a jukebox, a decorated pot) is not one.
//! - Smart Chute filter: right-click while sneaking with an item sets it; with the same item again
//!   it steps the amount through "any", "exactly 1" ... "exactly 64" ("up to N" and "any" are
//!   the first row of Create's value box, which can't be reached here but by editing the
//!   block entity's `UpTo` and `FilterAmount`); sneaking with an empty hand clears it. Create's
//!   filter slot and value box, the displayed filter item and Goggles' tooltip
//!   (`IHaveGoggleInformation`) are not drawn. Filter items (`FilterItem`, attribute and list
//!   filters) don't exist here.
//! - No air particles, no package handling (`PackageEntity`), no waterlogging, no advancements.
//! - Collision boxes are cut to the block's own cell.
//! - A chute is not drawn carrying its item except through a window or over a fan's pull; the
//!   item moves smoothly there as Create's does, but a blocked item is told to clients with one
//!   update, not tracked.

use create_kinetics::rules::chute::{self as rules, Around, Shape};
use create_kinetics::rules::fan::{self as fan, Aabb, Passage};
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntityRenderBoundingBoxEvent,
    BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, ContainerAcceptsEvent, DisplayContext,
    EquipmentSlot, Flow, Hooks, Host, ItemStack, LevelHost, ModBlockEntity, ModContainer,
    PlaceBlockEvent, RightClickBlockEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::chute_inv::{self as inv, Filter};
use crate::belt::BeltInput;
use crate::{float, int, set_float, util};

pub const CHUTE: &str = "create:chute";
pub const SMART_CHUTE: &str = "create:smart_chute";
const ENCASED_FAN: &str = "create:encased_fan";
const FUNNELS: [&str; 2] = ["create:andesite_funnel", "create:brass_funnel"];
/// `AllBlocks.INDUSTRIAL_IRON_BLOCK`, what encases a chute.
const INDUSTRIAL_IRON: &str = "create:industrial_iron_block";

const POSITION: &str = "ItemPosition";
const PULL: &str = "Pull";
const PUSH: &str = "Push";
const BOTTOM: &str = "BottomAirFlowDistance";
/// The item the saved position belongs to (its id): another one in the slot is a new arrival.
const TRACKED: &str = "Tracked";
/// The game time the position is as of, for clients.
const TICK: &str = "Tick";
/// The item waits half way through the block: clients stop it there.
const REST: &str = "Rest";
/// A neighbour changed: work the pull and push out again.
const RECALC: &str = "Recalc";
/// Set once the first look at the neighbours has been made.
const READY: &str = "Ready";
/// How far below the chute a belt or a depot a fan's air reaches is (0 for none): `beltBelow`.
const BELT_BELOW: &str = "BeltBelow";
/// The item as clients are sent it, an empty compound for none.
const ITEM: &str = "Item";

/// Ticks between full looks at the neighbours for the fans' pull and push.
const FLOW_CHECK: i64 = 20;
/// Ticks between looks at the inventory a chute takes from.
const INPUT_RATE: i64 = 2;
/// Ticks between a chute's looks at its shape.
const STATE_RATE: i64 = 4;
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

struct Chutes;

pub fn hooks() -> Box<dyn Hooks> {
    // The belt hands items to chutes through a table of blocks that take them.
    crate::belt::register_input(BeltInput {
        claims: |block| block == CHUTE || block == SMART_CHUTE,
        can_insert: |host, pos, _side| can_directly_insert(host, pos),
        insert: |host, pos, _side, item, simulate| {
            insert_stack(host, pos, &item.stack, simulate)
        },
    });
    Box::new(Chutes)
}

fn is_smart(block: &str) -> bool {
    block == SMART_CHUTE
}

fn is_chute(block: &str) -> bool {
    block == CHUTE || block == SMART_CHUTE
}

fn add(pos: Pos, dir: Dir) -> Pos {
    util::add(pos, dir.normal())
}

fn salt(pos: Pos) -> i64 {
    (i64::from(pos[0]) * 31 + i64::from(pos[1]) * 17 + i64::from(pos[2]) * 7).rem_euclid(1000)
}

/// `AbstractChuteBlock.getChuteFacing`: the facing of the chute at `pos`, none for another block.
fn chute_facing(host: &dyn Host, pos: Pos) -> Option<Dir> {
    let block = host.block(pos);
    if block == CHUTE {
        util::facing(host, pos, "facing")
    } else if block == SMART_CHUTE {
        Some(Dir::Down)
    } else {
        None
    }
}

fn shape_of(host: &dyn Host, pos: Pos) -> Option<Shape> {
    host.block_property(pos, "shape")
        .as_deref()
        .and_then(Shape::from_name)
}

/// `AbstractChuteBlock.isOpenChute`.
fn is_open(host: &dyn Host, pos: Pos) -> bool {
    match host.block(pos).as_str() {
        CHUTE => {
            util::facing(host, pos, "facing") == Some(Dir::Down)
                || shape_of(host, pos) == Some(Shape::Intersection)
        }
        SMART_CHUTE => true,
        _ => false,
    }
}

/// `SmartChuteBlockEntity.canActivate`: not powered by redstone (always for a plain chute).
fn can_activate(host: &dyn Host, pos: Pos) -> bool {
    host.block(pos) != SMART_CHUTE || host.block_property(pos, "powered").as_deref() == Some("false")
}

fn held(entity: &ModBlockEntity) -> Option<ItemStack> {
    entity.items.first().and_then(Clone::clone)
}

fn motion_of(nbt: &Compound) -> f32 {
    rules::item_motion(float(nbt, PUSH).unwrap_or(0.0), float(nbt, PULL).unwrap_or(0.0))
}

fn filter_of(host: &dyn Host, pos: Pos, nbt: &Compound) -> Filter {
    if host.block(pos) == SMART_CHUTE {
        inv::filter_of(nbt)
    } else {
        Filter { item: None, amount: rules::Amount::default() }
    }
}

/// `ChuteBlockEntity.canAcceptItem` (`SmartChuteBlockEntity.canAcceptItem` for a Smart Chute) of
/// the chute at `pos` whose block entity data is `entity`.
fn accepts(host: &dyn Host, pos: Pos, entity: &ModBlockEntity, stack: &ItemStack) -> bool {
    if held(entity).is_some() {
        return false;
    }
    if host.block(pos) == SMART_CHUTE {
        return can_activate(host, pos) && inv::filter_of(&entity.nbt).test(stack);
    }
    true
}

/// `canAcceptItem` of the chute at `pos`, read from the world.
fn chute_accepts(host: &dyn Host, pos: Pos, stack: &ItemStack) -> bool {
    host.mod_block_entity(pos)
        .is_some_and(|entity| entity.id == host.block(pos) && accepts(host, pos, &entity, stack))
}

/// The chutes that empty into the one at `pos` (`getInputChutes`): above it facing down, and
/// diagonally above on each side facing it.
fn input_chutes(host: &dyn Host, pos: Pos) -> Vec<Pos> {
    let above = add(pos, Dir::Up);
    let mut found = Vec::new();
    if chute_facing(host, above) == Some(Dir::Down) {
        found.push(above);
    }
    for side in rules::HORIZONTALS {
        let at = add(above, side);
        if chute_facing(host, at) == Some(side) {
            found.push(at);
        }
    }
    found
}

/// `getTargetChute`: the chute this one empties into, if the block there is one.
fn target_chute(host: &dyn Host, pos: Pos, facing: Dir) -> Option<Pos> {
    let mut at = add(pos, Dir::Down);
    if facing.axis() != create_kinetics::Axis::Y {
        at = add(at, facing.opposite());
    }
    is_chute(&host.block(at)).then_some(at)
}

// --- the DirectBeltInputBehaviour -------------------------------------------------------------

/// `ChuteBlockEntity.canDirectlyInsert`: whether a belt or a falling item can hand the chute at
/// `pos` something, from outside the chute's own tick.
fn can_directly_insert(host: &dyn Host, pos: Pos) -> bool {
    host.mod_block_entity(pos)
        .is_some_and(|entity| directly_insertable(host, pos, motion_of(&entity.nbt)))
}

/// `canDirectlyInsert` for a chute whose item motion is `motion`.
fn directly_insertable(host: &dyn Host, pos: Pos, motion: f32) -> bool {
    if !is_chute(&host.block(pos)) {
        return false;
    }
    if chute_facing(host, add(pos, Dir::Up)) == Some(Dir::Down) {
        return false;
    }
    if motion > 0.0 && input_chutes(host, pos).is_empty() {
        return false;
    }
    is_open(host, pos)
}

/// `ChuteItemHandler.insertItem` for the chute at `pos`: the stack in (limited to one stack's
/// worth), the rest handed back. Count 0 where it all went.
fn insert_stack(host: &mut dyn Host, pos: Pos, stack: &ItemStack, simulate: bool) -> ItemStack {
    if !chute_accepts(host, pos, stack) {
        return stack.clone();
    }
    // `ItemHelper.limitCountToMaxStackSize`.
    let limit = host.max_stack_size(&stack.item).unwrap_or(64);
    let mut put = stack.clone();
    put.count = stack.count.min(limit);
    let mut rest = stack.clone();
    rest.count -= put.count;
    if !simulate {
        let start = match host.mod_block_entity(pos) {
            Some(entity) if motion_of(&entity.nbt) < 0.0 => 1.0,
            _ => 0.0,
        };
        set_item_remote(host, pos, put, start);
    }
    rest
}

/// `ChuteBlockEntity.setItem(stack, position)` on another chute.
fn set_item_remote(host: &mut dyn Host, pos: Pos, stack: ItemStack, start: f32) {
    let now = util::game_time(host);
    host.change_mod_block_entity(pos, &mut |entity| put_item(entity, Some(stack.clone()), start, now));
    host.block_entity_sync(pos);
}

/// Puts `stack` in the slot and starts it at `start`.
fn put_item(entity: &mut ModBlockEntity, stack: Option<ItemStack>, start: f32, now: i64) {
    if entity.items.is_empty() {
        entity.items.push(None);
    }
    match &stack {
        Some(stack) => {
            entity.nbt.insert(TRACKED.into(), Tag::String((&*stack.item).into()));
            set_float(&mut entity.nbt, POSITION, start);
            entity.nbt.insert(TICK.into(), Tag::Long(now));
        }
        None => {
            util::remove(&mut entity.nbt, TRACKED);
        }
    }
    util::remove(&mut entity.nbt, REST);
    entity.items[0] = stack;
}

// --- the block entity's tick ----------------------------------------------------------------

/// What came of moving an item through the half-way mark.
enum Out {
    /// The item waits half way.
    Blocked,
    /// What the item was was handed on (or some of it): nothing more to do this tick.
    Gone,
    /// It may go on. At the end of the block: hand it to a chute or drop it.
    Free(End),
}

enum End {
    Chute(Pos, f32),
    DropBelow,
    DropAbove,
}

fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let block = host.block(pos);
    if !is_chute(&block) {
        return;
    }
    let smart = is_smart(&block);
    let now = util::game_time(host);
    let phase = now + salt(pos);
    let mut sync = false;

    if smart {
        if phase % 2 == 0 {
            sync_powered(host, pos);
        }
    } else if phase % STATE_RATE == 0 {
        refresh_state(host, pos);
    }
    let facing = if smart {
        Dir::Down
    } else {
        util::facing(host, pos, "facing").unwrap_or(Dir::Down)
    };

    // Pull and push from the fans and the chain.
    let flows = update_flows(host, pos, entity, facing, phase);
    sync |= flows;
    let motion = motion_of(&entity.nbt);

    // Air: how far below a fan reaches, items on the ground below, a belt below.
    if motion > 0.0 && (flows || phase % rules::AIR_CHECK_RATE == 0) {
        sync |= update_air_flow(host, pos, entity, motion);
    }

    // An item that arrived by a hopper, or went by one.
    sync |= track(entity, motion, now);

    if held(entity).is_none() {
        if motion > 0.0 && phase % rules::ENTITY_SEARCH == 0 {
            sync |= find_entities(host, pos, entity, now);
        }
        if motion > 0.0 && held(entity).is_none() {
            sync |= extract_from_belt(host, pos, entity, now);
        }
        if held(entity).is_none() {
            if phase % INPUT_RATE == 0 && can_activate(host, pos) {
                sync |= handle_input(host, pos, entity, motion, now);
            }
            if held(entity).is_none() && phase % 2 == 0 && directly_insertable(host, pos, motion) {
                sync |= pick_up_landed(host, pos, entity, motion, now);
            }
        }
        if sync {
            finish_sync(host, pos, entity, now);
        }
        return;
    }
    sync |= step(host, pos, entity, facing, motion, now);
    if sync {
        finish_sync(host, pos, entity, now);
    }
}

fn finish_sync(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, now: i64) {
    entity.nbt.insert(TICK.into(), Tag::Long(now));
    host.block_entity_sync(pos);
}

/// The Smart Chute's `powered` follows redstone (`SmartChuteBlock.tick`).
fn sync_powered(host: &mut dyn Host, pos: Pos) {
    let dimension = host.dimension();
    let powered = host.neighbor_signal(dimension, pos) > 0;
    let now = host.block_property(pos, "powered").as_deref() == Some("true");
    if powered != now {
        host.set_block_property(pos, "powered", if powered { "true" } else { "false" });
    }
}

/// An item in the slot that isn't the one the position belongs to arrived by a hopper (or the
/// chunk loading): it starts at the end it enters from. An empty slot forgets the item.
fn track(entity: &mut ModBlockEntity, motion: f32, now: i64) -> bool {
    let stack = held(entity);
    let tracked = match util::get(&entity.nbt, TRACKED) {
        Some(Tag::String(id)) => Some(id.to_string_lossy()),
        _ => None,
    };
    match (&stack, &tracked) {
        (Some(stack), t) if t.as_deref() != Some(&*stack.item) => {
            put_item(entity, Some(stack.clone()), if motion < 0.0 { 1.0 } else { 0.0 }, now);
            true
        }
        (None, Some(_)) => {
            put_item(entity, None, 0.0, now);
            true
        }
        _ => false,
    }
}

/// One tick of an item on its way (`ChuteBlockEntity.tick`).
fn step(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    facing: Dir,
    motion: f32,
    now: i64,
) -> bool {
    let Some(stack) = held(entity) else {
        return false;
    };
    let position = float(&entity.nbt, POSITION).unwrap_or(0.5);
    let mut next = position + motion;
    let mut rest = false;
    let mut sync = false;
    if motion < 0.0 {
        if next < 0.5 {
            match down_output(host, pos, entity, &stack, facing) {
                Out::Blocked => {
                    next = 0.5;
                    rest = true;
                }
                Out::Gone => return true,
                Out::Free(end) => {
                    if next < 0.0 {
                        if finish(host, pos, entity, &stack, end, motion, now) {
                            return true;
                        }
                        next = 0.5;
                        rest = true;
                    }
                }
            }
        }
    } else if motion > 0.0 && next > 0.5 {
        match up_output(host, pos, entity, &stack) {
            Out::Blocked => {
                next = 0.5;
                rest = true;
            }
            Out::Gone => return true,
            Out::Free(end) => {
                if next > 1.0 {
                    if finish(host, pos, entity, &stack, end, motion, now) {
                        return true;
                    }
                    next = 0.5;
                    rest = true;
                }
            }
        }
    }
    if float(&entity.nbt, POSITION) != Some(next) {
        set_float(&mut entity.nbt, POSITION, next);
    }
    let resting = util::has(&entity.nbt, REST);
    if rest != resting {
        if rest {
            entity.nbt.insert(REST.into(), Tag::Byte(1));
        } else {
            util::remove(&mut entity.nbt, REST);
        }
        sync = true;
    }
    sync
}

/// The item reached the end of the block and may leave: into the chute it empties into, or out
/// as an item entity. False where the chute it was to go into has taken something else since
/// (`handleDownwardOutput` asks again before it moves the item).
fn finish(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    stack: &ItemStack,
    end: End,
    motion: f32,
    now: i64,
) -> bool {
    match end {
        End::Chute(target, start) => {
            if !chute_accepts(host, target, stack) {
                return false;
            }
            set_item_remote(host, target, stack.clone(), start);
        }
        End::DropBelow => {
            let c = util::centre(pos);
            util::spawn_item(
                host,
                [c[0], c[1] + rules::DROP_BELOW, c[2]],
                stack,
                [0.0, rules::DROP_SPEED, 0.0],
                rules::PICKUP_DELAY,
            );
        }
        End::DropAbove => {
            let c = util::centre(pos);
            util::spawn_item(
                host,
                [c[0], c[1] + rules::DROP_ABOVE, c[2]],
                stack,
                [0.0, f64::from(motion) * 2.0, 0.0],
                rules::PICKUP_DELAY,
            );
        }
    }
    put_item(entity, None, 0.0, now);
    true
}

/// `ChuteBlockEntity.handleDownwardOutput`.
fn down_output(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    stack: &ItemStack,
    facing: Dir,
) -> Out {
    if !can_activate(host, pos) {
        return Out::Blocked;
    }
    let below = add(pos, Dir::Down);
    if !is_chute(&host.block(below)) {
        // `grabCapability(DOWN)`: an inventory below takes the item as it reaches it.
        if let Some(rest) = inv::insert(host, below, Dir::Up, stack, false) {
            if rest.count != stack.count {
                let now = util::game_time(host);
                put_item(
                    entity,
                    (rest.count > 0).then_some(rest),
                    float(&entity.nbt, POSITION).unwrap_or(0.5),
                    now,
                );
                return Out::Gone;
            }
            if facing == Dir::Down {
                return Out::Blocked;
            }
        }
    }
    if let Some(target) = target_chute(host, pos, facing) {
        return if chute_accepts(host, target, stack) {
            Out::Free(End::Chute(target, if facing == Dir::Down { 1.0 } else { 0.51 }))
        } else {
            Out::Blocked
        };
    }
    // Diagonal chutes cannot drop items.
    if facing.axis() != create_kinetics::Axis::Y {
        return Out::Blocked;
    }
    if FUNNELS.contains(&host.block(below).as_str())
        && util::facing(host, below, "facing") == Some(Dir::Down)
    {
        return Out::Blocked;
    }
    if solid_face(host, below, Dir::Up) {
        return Out::Blocked;
    }
    Out::Free(End::DropBelow)
}

/// `ChuteBlockEntity.handleUpwardOutput`.
fn up_output(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    stack: &ItemStack,
) -> Out {
    if !can_activate(host, pos) {
        return Out::Blocked;
    }
    let above = add(pos, Dir::Up);
    if is_open(host, pos) && !is_chute(&host.block(above)) {
        // `grabCapability(UP)`: an inventory above takes the item as it reaches it.
        if let Some(rest) = inv::insert(host, above, Dir::Down, stack, false) {
            if rest.count != stack.count {
                let now = util::game_time(host);
                put_item(
                    entity,
                    (rest.count > 0).then_some(rest),
                    float(&entity.nbt, POSITION).unwrap_or(0.5),
                    now,
                );
                return Out::Gone;
            }
            return Out::Blocked;
        }
    }
    let inputs = input_chutes(host, pos);
    let mut best: Option<(Pos, f32)> = None;
    for target in &inputs {
        if !chute_accepts(host, *target, stack) {
            continue;
        }
        let Some(other) = host.mod_block_entity(*target) else {
            continue;
        };
        let motion = motion_of(&other.nbt);
        if motion < 0.0 {
            continue;
        }
        if best.is_none_or(|(_, have)| have < motion) {
            best = Some((*target, motion));
        }
    }
    if let Some((target, _)) = best {
        return Out::Free(End::Chute(target, 0.0));
    }
    if FUNNELS.contains(&host.block(above).as_str())
        && util::facing(host, above, "facing") == Some(Dir::Up)
    {
        return Out::Blocked;
    }
    if solid_face(host, above, Dir::Down) || !inputs.is_empty() {
        return Out::Blocked;
    }
    Out::Free(End::DropAbove)
}

/// `Block.canSupportRigidBlock` / `BlockHelper.hasBlockSolidSide`: the face of the block at `pos`
/// on its side `side` is filled by its collision boxes.
fn solid_face(host: &dyn Host, pos: Pos, side: Dir) -> bool {
    const CELLS: usize = 16;
    let axis = side.axis().index();
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let mut covered = [[false; CELLS]; CELLS];
    for b in host.block_collision(pos) {
        let touches = if side.sign() > 0 {
            b[axis + 3] >= 1.0 - 1e-6
        } else {
            b[axis] <= 1e-6
        };
        if !touches {
            continue;
        }
        let cells = |lo: f64, hi: f64| {
            let lo = (lo.max(0.0) * CELLS as f64 + 1e-6).floor() as usize;
            let hi = (hi.min(1.0) * CELLS as f64 - 1e-6).ceil() as usize;
            lo..hi.min(CELLS)
        };
        for i in cells(b[u], b[u + 3]) {
            for j in cells(b[v], b[v + 3]) {
                covered[i][j] = true;
            }
        }
    }
    covered.iter().flatten().all(|c| *c)
}

// --- taking items in ------------------------------------------------------------------------

/// `ChuteBlockEntity.handleInputFromAbove` and `handleInputFromBelow`: takes what the inventory
/// above (while items go down) or below (while they rise) has.
fn handle_input(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    motion: f32,
    now: i64,
) -> bool {
    let (side, start, face) = if motion < 0.0 {
        (Dir::Up, 1.0, Dir::Down)
    } else {
        (Dir::Down, 0.0, Dir::Up)
    };
    let source = add(pos, side);
    // `grabCapability`: no chute is an inventory to take from.
    if is_chute(&host.block(source)) {
        return false;
    }
    let filter = filter_of(host, pos, &entity.nbt);
    let (amount, mode) = if host.block(pos) == SMART_CHUTE {
        // `SmartChuteBlockEntity.isExtracting`: a block that isn't a chute and can't be built
        // over is on the extracting side.
        let visible = filter.max_stack(host) > 1;
        rules::smart_extraction(filter.amount, visible)
    } else {
        (16, rules::Mode::UpTo)
    };
    let stack = inv::extract(host, source, face, mode, amount, &mut |_, candidate| {
        filter.test(candidate)
    });
    let Some(stack) = stack else {
        return false;
    };
    put_item(entity, Some(stack), start, now);
    true
}

/// `AbstractChuteBlock.updateEntityAfterFallOn`: an item lying on top of the chute goes in.
fn pick_up_landed(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    motion: f32,
    now: i64,
) -> bool {
    let [x, y, z] = pos.map(f64::from);
    let dimension = host.dimension();
    for found in host.item_entities(dimension, [x, y + 0.9, z], [x + 1.0, y + 1.5, z + 1.0]) {
        let [px, _, pz] = found.position;
        if !(x..x + 1.0).contains(&px) || !(z..z + 1.0).contains(&pz) {
            continue;
        }
        let resting = host
            .entity(found.id)
            .and_then(|e| host.entity_on_ground(e))
            .unwrap_or(false);
        if !resting || !accepts(host, pos, entity, &found.stack) {
            continue;
        }
        let limit = host.max_stack_size(&found.stack.item).unwrap_or(64);
        let mut put = found.stack.clone();
        put.count = put.count.min(limit);
        let mut rest = found.stack;
        rest.count -= put.count;
        host.set_item_entity(dimension, found.id, rest);
        put_item(entity, Some(put), if motion < 0.0 { 1.0 } else { 0.0 }, now);
        return true;
    }
    false
}

// --- the fans' air ----------------------------------------------------------------------------

/// The speed of the encased fan at `pos` if it faces `towards` (`EncasedFanBlock.FACING`).
fn fan_speed(host: &dyn Host, pos: Pos, towards: Dir) -> Option<f32> {
    if host.block(pos) != ENCASED_FAN || util::facing(host, pos, "facing") != Some(towards) {
        return None;
    }
    let entity = host.mod_block_entity(pos)?;
    Some(float(&entity.nbt, crate::SPEED).unwrap_or(0.0))
}

/// `ChuteBlockEntity.updatePull` and `updatePush` together, with what `calculatePull` and
/// `calculatePush` read. True when either changed (neighbours are told).
fn update_flows(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    facing: Dir,
    phase: i64,
) -> bool {
    let fan_near = host.block(add(pos, Dir::Up)) == ENCASED_FAN
        || host.block(add(pos, Dir::Down)) == ENCASED_FAN;
    let flagged = util::has(&entity.nbt, RECALC);
    let first = !util::has(&entity.nbt, READY);
    if !(first || flagged || phase % FLOW_CHECK == 0 || (fan_near && phase % 2 == 0)) {
        return false;
    }
    let inputs = input_chutes(host, pos);
    let pull = {
        let fan_above = fan_speed(host, add(pos, Dir::Up), Dir::Down);
        let pulls: Vec<f32> = inputs
            .iter()
            .map(|p| host.mod_block_entity(*p).map_or(0.0, |e| float(&e.nbt, PULL).unwrap_or(0.0)))
            .collect();
        rules::total_pull(fan_above, pulls)
    };
    let target = target_chute(host, pos, facing);
    let push = {
        let fan_below = fan_speed(host, add(pos, Dir::Down), Dir::Up);
        let from_target = target.and_then(|t| host.mod_block_entity(t)).map(|e| float(&e.nbt, PUSH).unwrap_or(0.0));
        let branches = target.map_or(1, |t| input_chutes(host, t).len());
        rules::total_push(fan_below, from_target, branches)
    };
    if flagged {
        util::remove(&mut entity.nbt, RECALC);
    }
    let changed = float(&entity.nbt, PULL) != Some(pull) || float(&entity.nbt, PUSH) != Some(push);
    if changed {
        set_float(&mut entity.nbt, PULL, pull);
        set_float(&mut entity.nbt, PUSH, push);
    }
    if first {
        entity.nbt.insert(READY.into(), Tag::Byte(1));
    }
    if changed || first {
        // The chute below and those above work theirs out again.
        let mut neighbours = inputs;
        neighbours.extend(target);
        for other in neighbours {
            host.change_mod_block_entity(other, &mut |e| {
                e.nbt.insert(RECALC.into(), Tag::Byte(1));
            });
        }
    }
    changed
}

/// `AirCurrent.getFlowLimit` for a chute's `max` reach downwards, with the world as the fan
/// reads it.
fn flow_limit(host: &dyn Host, pos: Pos, max: f32) -> f32 {
    let dimension = host.dimension();
    let level = host.level(dimension);
    fan::flow_limit(max, Dir::Down, |i| {
        let at = [pos[0], pos[1] - (i + 1), pos[2]];
        if !level.is_some_and(|level| level.is_loaded(host, at)) {
            return Passage::Unloaded;
        }
        let name = host.block(at);
        if host.block_name_in_tag(&name, "create:fan_transparent").unwrap_or(false) {
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
    })
}

/// `ChuteBlockEntity.updateAirFlow` (while items rise): how far below the chute a fan's pull
/// reaches and the belt or depot in it. True when something clients need changed.
fn update_air_flow(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, motion: f32) -> bool {
    if motion <= 0.0 {
        return false;
    }
    let speed = float(&entity.nbt, PULL).unwrap_or(0.0) - float(&entity.nbt, PUSH).unwrap_or(0.0);
    let mut max = rules::max_pull_distance(speed);
    if is_chute(&host.block(add(pos, Dir::Down))) {
        max = 0.0;
    }
    let mut limit = max;
    if limit > 0.0 {
        limit = flow_limit(host, pos, max);
    }
    let mut belt = 0;
    for i in 1..=(limit as i32 + 1) {
        let at = [pos[0], pos[1] - i, pos[2]];
        if holds_items_on_top(host, at) {
            belt = i;
            break;
        }
    }
    let bottom = limit.max(0.0);
    let mut changed = false;
    if float(&entity.nbt, BOTTOM) != Some(bottom) {
        set_float(&mut entity.nbt, BOTTOM, bottom);
        changed = true;
    }
    if int(&entity.nbt, BELT_BELOW) != belt {
        entity.nbt.insert(BELT_BELOW.into(), Tag::Int(belt));
    }
    changed
}

/// A `TransportedItemStackHandlerBehaviour`: a belt or a depot.
fn holds_items_on_top(host: &dyn Host, at: Pos) -> bool {
    let block = host.block(at);
    block == crate::belt::ID || block == crate::depot::ID
}

/// `ChuteBlockEntity.findEntities`: an item entity in the air below is pulled in.
fn find_entities(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, now: i64) -> bool {
    if !can_activate(host, pos) {
        return false;
    }
    let bottom = f64::from(float(&entity.nbt, BOTTOM).unwrap_or(0.0));
    let c = util::centre(pos);
    let min = [c[0] - 0.45, c[1] - bottom - 0.5 - 0.45, c[2] - 0.45];
    let max = [c[0] + 0.45, c[1] - 0.5 + 0.45, c[2] + 0.45];
    let dimension = host.dimension();
    for found in host.item_entities(dimension, min, max) {
        if !accepts(host, pos, entity, &found.stack) {
            continue;
        }
        // `itemEntity.getBoundingBox().getCenter().y - worldPosition.getY()`: an item is a
        // quarter of a block high.
        let start = (found.position[1] + 0.125 - f64::from(pos[1])) as f32;
        let mut gone = found.stack.clone();
        gone.count = 0;
        host.set_item_entity(dimension, found.id, gone);
        put_item(entity, Some(found.stack), start, now);
        return true;
    }
    false
}

/// `ChuteBlockEntity.extractFromBelt`: an item at the middle of the belt or the depot the fan
/// reaches is taken off it.
fn extract_from_belt(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, now: i64) -> bool {
    let below = int(&entity.nbt, BELT_BELOW);
    if below <= 0 || !can_activate(host, pos) {
        return false;
    }
    let at = [pos[0], pos[1] - below, pos[2]];
    let filter = filter_of(host, pos, &entity.nbt);
    let smart = host.block(pos) == SMART_CHUTE;
    let mut accept = |stack: &ItemStack| !smart || filter.test(stack);
    let taken = match host.block(at).as_str() {
        crate::belt::ID => crate::belt::take_centered(host, at, &mut accept),
        crate::depot::ID => take_from_depot(host, at, &mut accept),
        _ => None,
    };
    let Some(stack) = taken else {
        return false;
    };
    put_item(entity, Some(stack), -((below - 1) as f32), now);
    true
}

/// `DepotBehaviour.handleCenteredProcessingOnAllItems`: the held item, or one of the output
/// buffer, if `accept` takes it.
fn take_from_depot(host: &mut dyn Host, at: Pos, accept: &mut dyn FnMut(&ItemStack) -> bool) -> Option<ItemStack> {
    let entity = host.mod_block_entity(at)?;
    let slot = entity
        .items
        .iter()
        .position(|s| s.as_ref().is_some_and(|stack| accept(stack)))?;
    let stack = entity.items[slot].clone()?;
    host.change_mod_block_entity(at, &mut |e| {
        if slot < e.items.len() {
            e.items[slot] = None;
        }
    });
    host.block_entity_sync(at);
    Some(stack)
}

// --- the chute's shape ------------------------------------------------------------------------

/// `ChuteBlock.updateChuteState` for the chute at `pos`, written to its block state.
fn refresh_state(host: &mut dyn Host, pos: Pos) {
    if host.block(pos) != CHUTE {
        return;
    }
    let Some(facing) = util::facing(host, pos, "facing") else {
        return;
    };
    let Some(shape) = shape_of(host, pos) else {
        return;
    };
    let above = add(pos, Dir::Up);
    let mut feeding = [false; 4];
    for (i, side) in rules::HORIZONTALS.into_iter().enumerate() {
        let at = add(above, side);
        feeding[i] = host.block(at) == CHUTE && util::facing(host, at, "facing") == Some(side);
    }
    let around = Around {
        above_chute_facing: chute_facing(host, above),
        above_funnel_down: FUNNELS.contains(&host.block(above).as_str())
            && util::facing(host, above, "facing") == Some(Dir::Down),
        feeding,
        target_is_chute: target_chute(host, pos, facing).is_some(),
    };
    let (new_facing, new_shape) = rules::update_chute_state(facing, shape, &around);
    if new_facing != facing {
        host.set_block_property(pos, "facing", new_facing.name());
    }
    if new_shape != shape {
        host.set_block_property(pos, "shape", new_shape.name());
    }
}

// --- drawing ----------------------------------------------------------------------------------

/// `ChuteRenderer`: the item through a window, or in a fan's air.
fn draw(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let entity = event.entity;
    let stack = match util::get(&entity.nbt, ITEM) {
        Some(tag @ Tag::Compound(c)) if !c.is_empty() => util::stack_of(tag),
        _ => None,
    };
    let Some(stack) = stack else {
        event.draw.keep();
        return;
    };
    let property = |key: &str| host.block_state_property(event.state, key);
    if property("facing") != Some("down") {
        event.draw.keep();
        return;
    }
    let motion = motion_of(&entity.nbt);
    let since = (event.draw.game_time - util::long(&entity.nbt, TICK)) as f32 + event.draw.partial;
    let mut position = float(&entity.nbt, POSITION).unwrap_or(0.5) + motion * since.clamp(0.0, 40.0);
    // The item stops half way where something holds it, and at either end.
    let low = if motion < 0.0 { 0.0 } else { -4.0 };
    let high = if motion < 0.0 { 1.5 } else { 1.0 };
    position = position.clamp(low, high);
    if util::has(&entity.nbt, REST) {
        position = if motion < 0.0 { position.max(0.5) } else { position.min(0.5) };
    }
    let bottom = float(&entity.nbt, BOTTOM).unwrap_or(0.0);
    if property("shape") != Some("window") && (bottom == 0.0 || position > 0.5) {
        event.draw.keep();
        return;
    }
    let draw = &mut *event.draw;
    draw.push();
    draw.translate([0.5, position, 0.5]);
    draw.scale([0.5; 3]);
    draw.rotate_about([1.0, 0.0, 0.0], position * 180.0, [0.0; 3]);
    draw.rotate_about([0.0, 1.0, 0.0], position * 180.0, [0.0; 3]);
    host.draw_item(draw, &stack, DisplayContext::Fixed, None, 0);
    draw.pop();
}

fn container() -> ModContainer {
    ModContainer {
        slots: 1,
        faces: [(); 6].map(|()| vec![0]),
        inputs: vec![0],
        outputs: vec![0],
        menu: None,
    }
}

// --- the hooks --------------------------------------------------------------------------------

impl Hooks for Chutes {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        is_chute(event.block).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_chute(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_container(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        is_chute(event.block).then(container)
    }

    /// `ChuteItemHandler.insertItem`: nothing goes in while the chute holds something, and a Smart
    /// Chute takes only what its filter lets by, and not while redstone powers it.
    fn container_accepts(&self, host: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if !is_chute(&event.entity.id) {
            return Flow::Continue;
        }
        if event.slot != 0 || held(event.entity).is_some() {
            return Flow::Cancel;
        }
        if event.entity.id == SMART_CHUTE
            && !(inv::filter_of(&event.entity.nbt).test(event.stack) && can_activate(host, event.pos))
        {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if is_chute(&event.entity.id) {
            tick(host, event.pos, event.entity);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if !is_chute(&event.entity.id) {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        tag.insert(
            ITEM.into(),
            match held(event.entity) {
                Some(stack) => util::stack_tag(&stack),
                None => Tag::Compound(Compound::new()),
            },
        );
        for key in [POSITION, PULL, PUSH, BOTTOM] {
            set_float(&mut tag, key, float(nbt, key).unwrap_or(0.0));
        }
        tag.insert(TICK.into(), Tag::Long(util::long(nbt, TICK)));
        tag.insert(REST.into(), Tag::Byte(i8::from(util::has(nbt, REST))));
        Some(tag)
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block == CHUTE {
            draw(host, event);
        }
    }

    /// `ChuteBlockEntity.createRenderBoundingBox`: the block and three below it.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if is_chute(event.block) {
            event.bounds[1] = event.bounds[1].min(-3.0);
        }
    }

    /// `ChuteBlock.getStateForPlacement`: a chute placed on the side of a block faces that side
    /// (and turns back to face down at once where nothing is there for it to lead to).
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != CHUTE && &*context.item != SMART_CHUTE {
            return None;
        }
        if &*context.item == SMART_CHUTE {
            // `SmartChuteBlock.getStateForPlacement`: powered by what is beside it; checked
            // once placed.
            return None;
        }
        let face = util::dir(context.face);
        if rules::diagonal_placement(face, context.sneaking) {
            return level
                .state_with_property(event.state, "facing", face.name())
                .map(Some);
        }
        None
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        match &*event.stack.item {
            CHUTE => refresh_state(host, event.pos),
            SMART_CHUTE => sync_powered(host, event.pos),
            _ => {}
        }
    }

    /// The Wrench (`ChuteBlock.onWrenched`), an Industrial Iron Block (`ChuteBlock.useItemOn`),
    /// a chute clicked on its side with a chute (`ChuteItem.place`), and a filter set on a Smart
    /// Chute (the stand-in for its filter slot).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        let item = &*event.stack.item;
        if item == CHUTE && is_chute(&block) {
            return side_placement(host, event);
        }
        if !is_chute(&block) {
            return Flow::Continue;
        }
        if item == util::WRENCH {
            return util::wrench(host, pos, |host| {
                if block != CHUTE {
                    return;
                }
                let shape = shape_of(host, pos);
                let down = util::facing(host, pos, "facing") == Some(Dir::Down);
                if let Some(next) = shape.and_then(|s| rules::wrenched(s, down)) {
                    host.set_block_property(pos, "shape", next.name());
                }
            });
        }
        let sneaking = host.acting_player().is_some_and(|p| p.sneaking);
        if item == INDUSTRIAL_IRON && block == CHUTE {
            let shape = shape_of(host, pos);
            if matches!(shape, Some(Shape::Normal | Shape::Window)) {
                host.set_block_property(pos, "shape", Shape::Encased.name());
                host.play_sound("minecraft:block.netherite_block.hit", util::centre(pos), BLOCKS, 0.5, 1.05);
                return Flow::Cancel;
            }
            return Flow::Continue;
        }
        if block == SMART_CHUTE && sneaking {
            if let Some(line) = inv::click(host, pos, Some(event.stack)) {
                inv::tell(host, &line);
            }
            return Flow::Cancel;
        }
        Flow::Continue
    }

    /// `AbstractChuteBlock.useItemOn` with an empty hand: the item comes back to the player; a
    /// Smart Chute's filter is cleared by sneaking.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        if !is_chute(&block) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let uuid = player.uuid;
        if host.player_equipment(uuid, EquipmentSlot::MainHand).is_some() {
            return Flow::Continue;
        }
        if player.sneaking && host.player_equipment(uuid, EquipmentSlot::OffHand).is_some() {
            return Flow::Continue;
        }
        if block == SMART_CHUTE && player.sneaking {
            if util::first_click(host, uuid, pos)
                && let Some(line) = inv::click(host, pos, None)
            {
                inv::tell(host, &line);
            }
            return Flow::Cancel;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let Some(stack) = held(&entity) else {
            return Flow::Continue;
        };
        util::click_taken(host, uuid, pos);
        let now = util::game_time(host);
        host.change_mod_block_entity(pos, &mut |e| put_item(e, None, 0.0, now));
        host.block_entity_sync(pos);
        // `Inventory.placeItemBackInInventory`.
        if let Some(rest) = host.give_stack(uuid, stack) {
            util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
        }
        Flow::Cancel
    }
}

/// `ChuteItem.place` for a chute clicked on a horizontal side with a chute in hand: the new one
/// goes above the block beside it, leaning on the clicked chute; or an existing vertical chute
/// there is turned to lean on it.
fn side_placement(host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
    let Some(face) = Dir::from_normal(event.face) else {
        return Flow::Continue;
    };
    let sneaking = host.acting_player().is_some_and(|p| p.sneaking);
    if !rules::diagonal_placement(face, sneaking) {
        return Flow::Continue;
    }
    let beside = add(event.pos, face);
    let correct = add(beside, Dir::Up);
    let dimension = host.dimension();
    let there = host.block(correct);
    let replaceable = matches!(there.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
        || host.block_name_in_tag(&there, "minecraft:replaceable").unwrap_or(false);
    if replaceable {
        let Some(state) = host.default_state(CHUTE) else {
            return Flow::Continue;
        };
        if !host.set_block(dimension, correct, state, util::UPDATE_ALL) {
            return Flow::Cancel;
        }
        host.set_block_property(correct, "facing", face.name());
        refresh_state(host, correct);
        host.play_sound("minecraft:block.netherite_block.place", util::centre(correct), BLOCKS, 1.0, 1.0);
        if host.acting_player().is_some_and(|p| p.game_mode == util::SURVIVAL) {
            host.take_items(CHUTE, 1);
        }
        return Flow::Cancel;
    }
    if there == CHUTE && util::facing(host, correct, "facing") == Some(Dir::Down) {
        host.set_block_property(correct, "facing", face.name());
        refresh_state(host, correct);
        return Flow::Cancel;
    }
    // `InteractionResult.FAIL`.
    Flow::Cancel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_salt_spreads_chutes_over_the_ticks() {
        assert_ne!(salt([0, 0, 0]), salt([1, 0, 0]));
        assert!(salt([-5, -64, 100]) < 1000);
    }

    #[test]
    fn a_chute_tells_its_blocks_from_others() {
        assert!(is_chute(CHUTE) && is_chute(SMART_CHUTE));
        assert!(is_smart(SMART_CHUTE) && !is_smart(CHUTE));
        assert!(!is_chute("create:depot"));
    }
}
