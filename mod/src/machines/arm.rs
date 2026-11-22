//! The Mechanical Arm (`ArmBlock`, `ArmBlockEntity`, `ArmItem`, `ArmRenderer`): it takes items
//! from its input points and puts them into its output points. The cycle and the numbers are
//! `create_kinetics::rules::arm` and `arm_points`; this is the glue: it reads the block entity and
//! the world, asks the rules and writes what they answer. What each point does in the world is
//! `arm_world.rs`, choosing points with the item `arm_select.rs`, drawing `arm_draw.rs`.
//!
//! Choosing points: right-click blocks with the Mechanical Arm in the main hand before placing it
//! (once for an input, again for an output; a funnel, a blaze burner, a campfire, a respawn anchor
//! and the crushing wheels only take), left-click to forget one, then place the arm: it keeps what
//! is within 5 blocks (`ArmInteractionPointHandler`, `ArmPlacementPacket`).
//!
//! Known differences from Create, for the README:
//! - Create 6.0.10's arm has no point type for chests, barrels or Item Vaults: they need a funnel
//!   or a depot, here as there. The Mechanical Crafter, the Packager and the Track Station are not
//!   here to point at.
//! - There is no value box: the Wrench on the arm steps the distribution method (round robin,
//!   forced round robin, prefer first target) and names it on the action bar; sneaking picks the
//!   arm up. Clicking the arm with Engineer's Goggles puts them on it (`ArmBlock.useItemOn`); the
//!   client may also swap them onto the player's head, as Horizon predicts an equippable's use.
//! - The selection is kept on the server per player (Create's lives on the client and is sent
//!   back by a packet), shown as coloured dust at the corners of the chosen blocks instead of
//!   outlines (blue to take from, yellow to deposit to), and dropped when the arm leaves the hand.
//!   An arm's points are not shown again with the Wrench.
//! - A waiting arm looks for an input every second tick (`SEARCH_RATE`), not every tick.
//! - Redstone is polled each tick (`neighbor_signal`) instead of heard; unpowering looks for an
//!   input at once only while the arm is searching (Create also does mid-move, which can overwrite
//!   the stack it carries).
//! - The progress of a move is derived from `MovementProgress` at `SyncTime` and the speed, so it
//!   is written only when the arm decides something or its speed changes; after a reload a move
//!   that was under way is as far along as the time says. `lastInputIndex` and `lastOutputIndex`
//!   are saved (`LastInputIndex`, `LastOutputIndex`); Create forgets them.
//! - A belt only holds the item the arm reaches for while it moves, and an arm can't take from a
//!   stopped belt. A funnel's flap doesn't swing for it. The Saw and the Crushing Wheels are
//!   deposit points here (Create lets an arm take from a saw's slot).
//! - The arm's saved `InteractionPoints` are not sent to clients (nothing client-side uses them),
//!   no "no targets" hint, no advancements.
//! - Campfire, jukebox and composter points work through Horizon's container and fake-player
//!   calls (`arm_world.rs`) and are not yet tried in game; a campfire's recipes, the composter's
//!   items and the records are baked from the game's data (`rules/arm_items.rs`).

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::arm::{self as rules, Angles, Arm, Motion, Phase, SelectionMode};
use create_kinetics::rules::arm_points::{Mode, Point, PointType};
use create_kinetics::{Axis, Kind, Node, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, DestroyBlockEvent,
    EquipmentSlot, Flow, Hooks, Host, ItemStack, LevelHost, ModBlockEntity, RightClickBlockEvent,
    StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::arm_world::{self, ArmWorld};
use crate::{belt, float, int, kinetic, util};

pub const ID: &str = "create:mechanical_arm";
pub(super) const MODEL_COG: &str = "create:block/mechanical_arm/cog";
pub(super) const MODELS: [&str; 7] = [
    "create:block/mechanical_arm/base",
    "create:block/mechanical_arm/lower_body",
    "create:block/mechanical_arm/upper_body",
    "create:block/mechanical_arm/claw_base",
    "create:block/mechanical_arm/claw_base_goggles",
    "create:block/mechanical_arm/upper_claw_grip",
    "create:block/mechanical_arm/lower_claw_grip",
];

// `ArmBlockEntity.write`'s keys.
const PHASE: &str = "Phase";
const POWERED: &str = "Powered";
pub(super) const GOGGLES: &str = "Goggles";
pub(super) const HELD: &str = "HeldItem";
const TARGET_INDEX: &str = "TargetPointIndex";
const PROGRESS: &str = "MovementProgress";
pub(super) const POINTS: &str = "InteractionPoints";
/// `ScrollOptionBehaviour`'s key: the selection mode's ordinal.
const SCROLL: &str = "ScrollValue";
// Ours: when `MovementProgress` was true, the round robin positions, and the angles clients move
// between (`previousTarget`, `previousBaseAngle`, the targeted point's angles).
const SYNC_TIME: &str = "SyncTime";
const LAST_INPUT: &str = "LastInputIndex";
const LAST_OUTPUT: &str = "LastOutputIndex";
const PREVIOUS: &str = "PreviousAngles";
const PREVIOUS_BASE: &str = "PreviousBase";
const TARGET: &str = "TargetAngles";
/// `SoundSource.BLOCKS` and `PLAYERS`.
const BLOCKS: usize = 4;

struct Arms;

pub fn hooks() -> Box<dyn Hooks> {
    // A belt holds the item an arm reaches for.
    belt::register_hold(arm_world::holds);
    Box::new(Arms)
}

/// The arms that have filtered their points once since the server started (`updateInteractionPoints`).
static STARTED: LazyLock<Mutex<HashSet<(String, Pos)>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

fn started() -> std::sync::MutexGuard<'static, HashSet<(String, Pos)>> {
    STARTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// --- the saved state ----------------------------------------------------------------------------

/// What a block entity saves of the cycle.
pub(super) struct Saved {
    pub arm: Arm<ItemStack>,
    /// `MovementProgress` as it was at `since`.
    pub start: f32,
    pub since: i64,
    pub motion: Motion,
}

pub(super) fn floats_of(nbt: &Compound, key: &str) -> Option<[f32; 4]> {
    let Some(Tag::List(list)) = util::get(nbt, key) else {
        return None;
    };
    let mut out = [0.0; 4];
    if list.len() != 4 {
        return None;
    }
    for (slot, tag) in out.iter_mut().zip(list) {
        let Tag::Float(bits) = tag else {
            return None;
        };
        *slot = f32::from_bits(*bits);
    }
    Some(out)
}

fn floats_tag(values: Option<[f32; 4]>) -> Tag {
    Tag::List(values.map_or_else(Vec::new, |v| v.iter().map(|f| Tag::Float(f.to_bits())).collect()))
}

/// The state saved in `nbt` (a client's has what the update tag sent). `now` stands in for a
/// missing `SyncTime`.
pub(super) fn read_saved(nbt: &Compound, now: i64) -> Saved {
    let held = util::get(nbt, HELD).and_then(util::stack_of);
    let phase = match util::get(nbt, PHASE) {
        Some(Tag::String(name)) => Phase::from_name(&name.to_string_lossy()),
        _ => Phase::SearchInputs,
    };
    let arm = Arm {
        phase,
        index: if util::has(nbt, TARGET_INDEX) { int(nbt, TARGET_INDEX) } else { -1 },
        progress: float(nbt, PROGRESS).unwrap_or(0.0),
        held,
        last_input: if util::has(nbt, LAST_INPUT) { int(nbt, LAST_INPUT) } else { -1 },
        last_output: if util::has(nbt, LAST_OUTPUT) { int(nbt, LAST_OUTPUT) } else { -1 },
        mode: SelectionMode::from_ordinal(int(nbt, SCROLL)),
        locked: util::boolean(nbt, POWERED),
    };
    let motion = Motion {
        previous: floats_of(nbt, PREVIOUS).map_or(rules::NO_TARGET, Angles::from_array),
        previous_base: float(nbt, PREVIOUS_BASE).unwrap_or(0.0),
        target: floats_of(nbt, TARGET).map(Angles::from_array),
    };
    Saved {
        start: arm.progress,
        arm,
        since: if util::has(nbt, SYNC_TIME) { util::long(nbt, SYNC_TIME) } else { now },
        motion,
    }
}

fn held_tag(held: &Option<ItemStack>) -> Tag {
    held.as_ref().map_or_else(|| Tag::Compound(Compound::new()), util::stack_tag)
}

/// Writes the cycle's state, with `progress` as of `now`, key by key where it differs.
fn write_saved(nbt: &mut Compound, arm: &Arm<ItemStack>, motion: &Motion, now: i64) {
    let put = |nbt: &mut Compound, key: &str, value: Tag| {
        if util::differs(nbt, key, &value) {
            nbt.insert(key.into(), value);
        }
    };
    put(nbt, PHASE, Tag::String(arm.phase.name().into()));
    put(nbt, POWERED, Tag::Byte(i8::from(arm.locked)));
    put(nbt, HELD, held_tag(&arm.held));
    put(nbt, TARGET_INDEX, Tag::Int(arm.index));
    put(nbt, LAST_INPUT, Tag::Int(arm.last_input));
    put(nbt, LAST_OUTPUT, Tag::Int(arm.last_output));
    put(nbt, SCROLL, Tag::Int(arm.mode.ordinal()));
    put(nbt, PROGRESS, Tag::Float(arm.progress.to_bits()));
    put(nbt, SYNC_TIME, Tag::Long(now));
    put(nbt, PREVIOUS, floats_tag(Some(motion.previous.to_array())));
    put(nbt, PREVIOUS_BASE, Tag::Float(motion.previous_base.to_bits()));
    put(nbt, TARGET, floats_tag(motion.target.map(Angles::to_array)));
}

/// `ArmInteractionPoint.serialize`: `Type`, `Pos` (relative to the arm) and `Mode`.
pub(super) fn points_tag(points: &[Point], arm: Pos) -> Tag {
    Tag::List(
        points
            .iter()
            .map(|p| {
                let mut tag = Compound::new();
                tag.insert("Type".into(), Tag::String(p.ty.id().into()));
                let at = (0..3).map(|i| p.pos[i] - arm[i]).collect();
                tag.insert("Pos".into(), Tag::IntArray(at));
                tag.insert("Mode".into(), Tag::String(p.mode.name().into()));
                Tag::Compound(tag)
            })
            .collect(),
    )
}

/// `ArmInteractionPoint.deserialize`, less the check that the block is still there.
fn read_points(nbt: &Compound, arm: Pos) -> Vec<Point> {
    let Some(Tag::List(list)) = util::get(nbt, POINTS) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|tag| {
            let Some(Tag::String(id)) = tag.get("Type") else {
                return None;
            };
            let ty = PointType::from_id(&id.to_string_lossy())?;
            let Some(Tag::IntArray(at)) = tag.get("Pos") else {
                return None;
            };
            let [x, y, z] = at.as_slice() else {
                return None;
            };
            let mode = match tag.get("Mode") {
                Some(Tag::String(name)) => Mode::from_name(&name.to_string_lossy()),
                _ => Mode::Deposit,
            };
            Some(Point { ty, pos: [arm[0] + x, arm[1] + y, arm[2] + z], mode })
        })
        .collect()
}

fn ceiling_of(host: &dyn Host, pos: Pos) -> bool {
    host.block_property(pos, "ceiling").as_deref() == Some("true")
}

// --- the tick --------------------------------------------------------------------------------------

/// `ArmBlockEntity.initInteractionPoints`: once the area is loaded, the points whose block is no
/// longer there are forgotten. True once done.
fn init_points(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) -> bool {
    let dimension = host.dimension().to_owned();
    if started().contains(&(dimension.clone(), pos)) {
        return true;
    }
    // `isAreaActuallyLoaded(worldPosition, range + 1)`.
    let reach = rules::RANGE + 1;
    for cx in (pos[0] - reach) >> 4..=(pos[0] + reach) >> 4 {
        for cz in (pos[2] - reach) >> 4..=(pos[2] + reach) >> 4 {
            if !host.chunk_is_loaded(&dimension, [cx, cz]) {
                return false;
            }
        }
    }
    let saved = read_points(&entity.nbt, pos);
    let ceiling = ceiling_of(host, pos);
    let mut world = ArmWorld::new(host, pos, ceiling, &saved);
    let kept: Vec<Point> = saved.iter().copied().filter(|p| world.valid(p)).collect();
    if kept.len() != saved.len() {
        entity.nbt.insert(POINTS.into(), points_tag(&kept, pos));
        host.block_entity_sync(pos);
    }
    started().insert((dimension, pos));
    true
}

fn salt(pos: Pos) -> i64 {
    (i64::from(pos[0]) * 31 + i64::from(pos[1]) * 17 + i64::from(pos[2]) * 7).rem_euclid(1000)
}

fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let old_speed = kinetic::speed(entity);
    // `ArmBlock implements ICogWheel` on the Y axis, costing 2 su per RPM.
    let node = Node { impact: rules::IMPACT, ..Node::passive(Kind::Cogwheel(Axis::Y)) };
    let Some((speed, speed_changed)) = kinetic::tick(host, pos, entity, node) else {
        return; // The network jammed and the block broke.
    };
    let now = util::game_time(host);
    if speed_changed {
        host.block_entity_sync(pos);
    }
    if now % 200 == salt(pos) % 200 {
        arm_world::prune(now);
    }
    if !init_points(host, pos, entity) {
        return;
    }
    let mut saved = read_saved(&entity.nbt, now);
    // Where the move was a tick ago, at the speed it had.
    saved.arm.progress = rules::progress_at(saved.start, saved.since, old_speed, now - 1, 0.0);

    // `ArmBlock.neighborChanged`: the signal the neighbour event left (`signals`).
    let powered = crate::signals::read(host, &mut entity.nbt, pos) > 0;
    let lazy_due = (now + salt(pos)).rem_euclid(rules::LAZY_PERIOD) == 0;
    let search_due = (now + salt(pos)).rem_euclid(rules::SEARCH_RATE) == 0;
    let redstone_changed = powered != saved.arm.locked;
    if !speed_changed
        && !redstone_changed
        && rules::may_skip(saved.arm.phase, saved.arm.progress, search_due, lazy_due)
    {
        return;
    }

    let points = read_points(&entity.nbt, pos);
    let ceiling = ceiling_of(host, pos);
    let before = (saved.arm.phase, saved.arm.index);
    let mut world = ArmWorld::new(host, pos, ceiling, &points);
    let left = rules::targeted(&saved.arm, &world).map(|i| match saved.arm.phase {
        Phase::MoveToInput => world.inputs[i],
        _ => world.outputs[i],
    });
    let left = left.map(|p| world.angles(&p));
    let mut events = rules::redstone_update(&mut saved.arm, &mut world, powered);
    let ran = rules::tick(&mut saved.arm, &mut world, rules::rate(speed), lazy_due);
    events.changed |= ran.changed;
    events.picked_up |= ran.picked_up;
    let target = rules::targeted(&saved.arm, &world).map(|i| match saved.arm.phase {
        Phase::MoveToInput => world.inputs[i],
        _ => world.outputs[i],
    });
    let target = target.map(|p| world.angles(&p));

    if !(events.changed || speed_changed) {
        return;
    }
    if (saved.arm.phase, saved.arm.index) != before {
        saved.motion = saved.motion.after(left, target);
    }
    write_saved(&mut entity.nbt, &saved.arm, &saved.motion, now);
    host.block_entity_sync(pos);
    if events.picked_up {
        let pitch = 0.5 + (util::random() % 1000) as f32 / 4000.0;
        host.play_sound("minecraft:entity.item.pickup", util::centre(pos), BLOCKS, 0.125, pitch);
    }
}

/// What a client is sent (`ArmBlockEntity.write` for a client, less the points).
fn update_tag(entity: &ModBlockEntity) -> Compound {
    let nbt = &entity.nbt;
    let mut tag = kinetic::update_tag(entity);
    for key in [PHASE, POWERED, GOGGLES, TARGET_INDEX, PROGRESS, SYNC_TIME, PREVIOUS_BASE, SCROLL] {
        if let Some(value) = util::get(nbt, key) {
            tag.insert(key.into(), value.clone());
        }
    }
    // The goggles' hint (`ArmBlockEntity.addToTooltip`): no inputs and no outputs chosen.
    let targets = matches!(util::get(nbt, POINTS), Some(Tag::List(points)) if !points.is_empty());
    tag.insert("HasTargets".into(), Tag::Byte(i8::from(targets)));
    // Always sent, empty when there is none, so a client drops what it had.
    tag.insert(HELD.into(), util::get(nbt, HELD).cloned().unwrap_or_else(|| Tag::Compound(Compound::new())));
    for key in [PREVIOUS, TARGET] {
        tag.insert(key.into(), util::get(nbt, key).cloned().unwrap_or(Tag::List(Vec::new())));
    }
    tag
}

impl Hooks for Arms {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(MODEL_COG.to_owned());
        event.models.extend(MODELS.iter().map(|m| (*m).to_owned()));
    }

    fn block_entity(&self, host: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            // The constructor: searching, nothing held, progress 0.
            let mut nbt = kinetic::new_entity();
            let arm: Arm<ItemStack> = Arm::new();
            write_saved(&mut nbt, &arm, &Motion::REST, util::game_time(host));
            nbt.insert(GOGGLES.into(), Tag::Byte(0));
            nbt.insert(POINTS.into(), Tag::List(Vec::new()));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == ID {
            tick(host, event.pos, event.entity);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        (event.entity.id == ID).then(|| update_tag(event.entity))
    }

    /// `ArmBlockEntity.createRenderBoundingBox().inflate(3)`.
    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block == ID {
            for i in 0..3 {
                event.bounds[i] -= 3.0;
                event.bounds[i + 3] += 3.0;
            }
        }
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block == ID {
            super::arm_draw::draw(host, event);
        }
    }

    /// `ArmBlock.getStateForPlacement`: hung from the ceiling when placed on the underside of a block.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if &*event.context.item != ID {
            return None;
        }
        let ceiling = util::dir(event.context.face) == create_kinetics::Dir::Down;
        level
            .state_with_property(event.state, "ceiling", if ceiling { "true" } else { "false" })
            .map(Some)
    }

    /// `ArmBlock.useItemOn`: Engineer's Goggles put the goggles on the arm (the item stays in the
    /// hand); any other click gives back the stack the arm holds.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let (main, off) = (
            host.player_equipment(player.uuid, EquipmentSlot::MainHand),
            host.player_equipment(player.uuid, EquipmentSlot::OffHand),
        );
        // Sneaking with anything in either hand skips the block's own use.
        if player.sneaking && (main.is_some() || off.is_some()) {
            return Flow::Continue;
        }
        // The off hand's turn of a click that already gave the stack back.
        if util::click_repeated(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let is_goggles = |stack: &Option<ItemStack>| {
            stack.as_ref().is_some_and(|s| &*s.item == "create:goggles")
        };
        if is_goggles(&main) || (main.is_none() && is_goggles(&off)) {
            let has = host.mod_block_entity(pos).is_some_and(|e| util::boolean(&e.nbt, GOGGLES));
            if !has {
                util::click_taken(host, player.uuid, pos);
                host.change_mod_block_entity(pos, &mut |e| {
                    e.nbt.insert(GOGGLES.into(), Tag::Byte(1));
                });
                host.block_entity_sync(pos);
                return Flow::Cancel;
            }
        }
        let Some(held) = host
            .mod_block_entity(pos)
            .and_then(|e| util::get(&e.nbt, HELD).and_then(util::stack_of))
        else {
            return Flow::Continue;
        };
        util::click_taken(host, player.uuid, pos);
        // `Inventory.placeItemBackInInventory`: what does not fit drops at the player.
        if let Some(rest) = host.give_stack(player.uuid, held) {
            util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
        }
        // `be.heldItem = EMPTY; be.phase = SEARCH_INPUTS`.
        host.change_mod_block_entity(pos, &mut |e| {
            e.nbt.insert(HELD.into(), Tag::Compound(Compound::new()));
            e.nbt.insert(PHASE.into(), Tag::String(Phase::SearchInputs.name().into()));
        });
        host.block_entity_sync(pos);
        Flow::Cancel
    }

    /// The Wrench only picks the arm up when sneaking (`IWrenchable`; the distribution method is the value box,
    /// `value_box.rs`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if &*event.stack.item != util::WRENCH || host.block(pos) != ID {
            return Flow::Continue;
        }
        util::wrench(host, pos, |_| {})
    }

    /// `ArmBlockEntity.destroy`: what it holds drops.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let held = host
            .mod_block_entity(pos)
            .and_then(|e| util::get(&e.nbt, HELD).and_then(util::stack_of));
        if let Some(held) = held {
            util::spawn_item(host, util::centre(pos), &held, [0.0; 3], 10);
        }
        let dimension = host.dimension().to_owned();
        started().remove(&(dimension, pos));
        Flow::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_state_reads_back() {
        let mut nbt = Compound::new();
        let mut arm: Arm<ItemStack> = Arm::new();
        arm.phase = Phase::MoveToOutput;
        arm.index = 2;
        arm.progress = 0.25;
        arm.last_input = 1;
        arm.mode = SelectionMode::PreferFirst;
        arm.held = Some(ItemStack::new("minecraft:iron_ingot", 5));
        let target = Angles { base: 10.0, lower: 20.0, upper: 30.0, head: 40.0 };
        let motion = Motion { previous: rules::NO_TARGET, previous_base: 7.0, target: Some(target) };
        write_saved(&mut nbt, &arm, &motion, 99);
        let back = read_saved(&nbt, 0);
        assert_eq!((back.arm.phase, back.arm.index, back.arm.last_input), (Phase::MoveToOutput, 2, 1));
        assert_eq!((back.start, back.since, back.arm.mode), (0.25, 99, SelectionMode::PreferFirst));
        assert_eq!(back.arm.held.map(|s| s.count), Some(5));
        assert_eq!(back.motion, motion);
        // A bare entity is a new arm.
        let fresh = read_saved(&Compound::new(), 5);
        assert_eq!((fresh.arm.phase, fresh.arm.index, fresh.since), (Phase::SearchInputs, -1, 5));
    }

    #[test]
    fn the_points_save_relative_to_the_arm_and_read_back() {
        let points = vec![
            Point { ty: PointType::Depot, pos: [11, 5, 3], mode: Mode::Take },
            Point { ty: PointType::Funnel, pos: [9, 6, 4], mode: Mode::Deposit },
        ];
        let mut nbt = Compound::new();
        nbt.insert(POINTS.into(), points_tag(&points, [10, 5, 4]));
        assert_eq!(read_points(&nbt, [10, 5, 4]), points);
        // Moved with a block: the same list is read at the new place.
        assert_eq!(read_points(&nbt, [0, 0, 0])[0].pos, [1, 0, -1]);
    }
}
