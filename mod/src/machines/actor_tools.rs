//! What each actor does on a moving structure: the Drill and the Saw (`DrillMovementBehaviour`,
//! `SawMovementBehaviour`), the Plough (`PloughMovementBehaviour`), the Harvester
//! (`HarvesterMovementBehaviour`), the Deployer (`DeployerMovementBehaviour`) and the Funnels
//! (`FunnelMovementBehaviour`) and, in `roller_actor.rs`, the Roller (`RollerMovementBehaviour`). The engine hook is
//! `actor.rs`; the rules are `create_kinetics::rules::{actor, harvester, plough, roller}`. This file reads the world, asks the rules
//! and applies the answer.
//!
//! Known differences from Create:
//! - Drills, Saws, Ploughs and Rollers hurt what stands in the cell they enter (the hit is
//!   `damage_entities`: `clamp(6 * |motion|^0.4 + 1, 2, 10)`) and throw mobs along the structure's
//!   motion; a player is hurt but not thrown (a server mod can't move a player). Entities are looked
//!   for in a cell the structure enters, not in the contraption's own bounding box
//!   (`isPassengerOfSameVehicle`: nothing rides a structure here).
//! - No hit sound at each crack stage (the host can't name a block's sound) and no
//!   `DrillActorVisual` particles; the head turns at the structure's speed.
//! - A Saw doesn't fell a tree the way `TreeCutter.findDynamicTree` does for another mod's trees.
//! - The Plough's hoe is not a fake player's: it tills what `HoeItem.TILLABLES` lists and plays the
//!   sound, nothing else.
//! - The Deployer's fake player is the hand the stationary Deployer has (`deployer_use.rs`): see
//!   there. Its hand (`HeldItem`, the overflow) is saved in the block's data when the structure
//!   stops. It can't use a Schematic as a printer on a structure.

use create_kinetics::rules::actor::{self as rules, Kind, Target};
use create_kinetics::rules::deployer::{self as deployer_rules, Mode};
use create_kinetics::rules::drill as drill_rules;
use create_kinetics::rules::harvester::{self as harvester, DEFAULTS, Left, Plant, Reap};
use create_kinetics::rules::plough::{self as plough, Cell};
use create_kinetics::rules::contraption::State;
use create_kinetics::{Dir, Pos};
use crate::compat::{Compound, Hooks, Host, ItemStack, Tag};

use super::actor::{Actor, Cx};
use super::chute_inv;
use super::contraption as engine;
use super::deployer_use::{self as hand, Crack, Items};
use super::saw;
use crate::util;

struct Shared;

impl Hooks for Shared {}

/// A module of shared code: no hooks of its own.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `BlockSoundGroup` ordinal of `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// `BlockBreakingMovementBehaviour`'s wait after a falling block (`WaitingTicks`).
pub(super) const FALLING_WAIT: i32 = 10;

/// What an actor remembers beyond its context and breaker.
#[derive(Default)]
pub(super) enum Extra {
    #[default]
    None,
    Deployer(Box<Hand>),
    /// A Redstone Contact's `lastContact`.
    Contact(Option<Pos>),
    /// A bell's `temporaryData`: the elevator's `moved` flag, or a Haunted Bell's recharge.
    Bell { moved: bool, recharge: Option<i32> },
}

/// `DeployerFakePlayer`'s state: what is in its hand, the block it is mining, the stall's timer.
pub(super) struct Hand {
    items: Items,
    crack: Option<Crack>,
    timer: i32,
}

// --- reading the world ------------------------------------------------------------------------------------

fn state_at(host: &dyn Host, pos: Pos) -> State {
    host.level(host.dimension())
        .and_then(|level| crate::compat::state_text(host, level, pos))
        .map_or_else(|| State::new("minecraft:air"), |text| State::parse(&text))
}

fn in_tag(host: &dyn Host, block: &str, tag: &str) -> bool {
    host.block_name_in_tag(block, tag).unwrap_or(false)
}

fn collision_empty(host: &dyn Host, pos: Pos) -> bool {
    host.block_collision(pos).is_empty()
}

fn below(pos: Pos) -> Pos {
    [pos[0], pos[1] - 1, pos[2]]
}

// --- the dispatch -----------------------------------------------------------------------------------------

/// `visitNewPosition`.
pub(super) fn visit(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos) {
    match actor.kind {
        Kind::Drill | Kind::Saw | Kind::Plough => {
            breaker_visit(cx, actor, cell);
            if actor.kind == Kind::Saw {
                saw_visit(cx, actor);
            }
            if actor.kind == Kind::Plough {
                plough_till(cx, actor, cell);
            }
        }
        Kind::Harvester => harvest(cx, actor, cell),
        Kind::Deployer => deployer_visit(cx, actor, cell),
        Kind::Funnel { brass } => funnel(cx, actor, cell, brass),
        Kind::Roller => super::roller_actor::visit(cx, actor, cell),
        Kind::Seat | Kind::Controls | Kind::Contact | Kind::Bell(_) | Kind::Campfire | Kind::Dispenser { .. } => {
            super::actor_more::visit(cx, actor, cell);
        }
    }
}

/// `tick`.
pub(super) fn tick(cx: &mut Cx<'_>, actor: &mut Actor) {
    match actor.kind {
        Kind::Drill | Kind::Saw | Kind::Plough | Kind::Roller => breaker_tick(cx, actor),
        Kind::Deployer => deployer_tick(cx, actor),
        Kind::Harvester | Kind::Funnel { .. } => {}
        Kind::Seat | Kind::Controls | Kind::Contact | Kind::Bell(_) | Kind::Campfire | Kind::Dispenser { .. } => super::actor_more::tick(cx, actor),
    }
}

/// `onSpeedChanged`: only the bells use it.
pub(super) fn speed_changed(cx: &mut Cx<'_>, actor: &mut Actor, old: [f64; 3]) {
    super::actor_more::speed_changed(cx, actor, old);
}

/// `stopMoving`.
pub(super) fn stop(cx: &mut Cx<'_>, actor: &mut Actor) {
    match actor.kind {
        Kind::Drill | Kind::Saw | Kind::Plough | Kind::Roller => cancel_stall(cx, actor),
        Kind::Deployer => deployer_stop(cx, actor),
        Kind::Harvester | Kind::Funnel { .. } => {}
        Kind::Seat | Kind::Controls | Kind::Contact | Kind::Bell(_) | Kind::Campfire | Kind::Dispenser { .. } => super::actor_more::stop(cx, actor),
    }
}

// --- block breaking (`BlockBreakingMovementBehaviour`) ---------------------------------------------------------

/// `canBreak` of each breaker.
pub(super) fn can_break(host: &dyn Host, kind: Kind, pos: Pos) -> bool {
    let block = host.block(pos);
    if kind == Kind::Plough {
        let below_block = host.block(below(pos));
        let cell = Cell {
            air: drill_rules::is_air(&block),
            farmland_below: create_kinetics::rules::actor_facts::FARMLAND.contains(&&*below_block),
            liquid: drill_rules::is_liquid(&block),
            bubble_column: block == "minecraft:bubble_column",
            blacklisted: in_tag(host, &block, "create:plough_blacklist"),
            whitelisted: in_tag(host, &block, "create:plough_whitelist"),
            collision_empty: collision_empty(host, pos),
        };
        return plough::can_break(&cell);
    }
    let hardness = drill_rules::hardness(&block);
    let non_breakable = in_tag(host, &block, "create:non_breakable");
    // `BlockBreakingKineticBlockEntity.isBreakable`.
    if !drill_rules::is_breakable(&block, hardness, non_breakable) {
        return false;
    }
    // `!state.getCollisionShape().isEmpty() && !AllTags.AllBlockTags.TRACKS.matches(state)`.
    let solid = || !collision_empty(host, pos) && !in_tag(host, &block, "create:tracks");
    match kind {
        Kind::Drill => solid(),
        // `SawMovementBehaviour.canBreak`: `super.canBreak(..) && isSawable`.
        Kind::Saw => saw::sawable(host, &block),
        Kind::Roller => {
            let portal_beside = Dir::ALL
                .iter()
                .any(|side| in_tag(host, &host.block(util::add(pos, side.normal())), "minecraft:portals"));
            !portal_beside && solid()
        }
        _ => false,
    }
}

fn target_of(host: &dyn Host, kind: Kind, pos: Pos) -> Target {
    let block = host.block(pos);
    Target { hardness: drill_rules::hardness(&block), breakable: can_break(host, kind, pos) }
}

/// `BlockBreakingMovementBehaviour.visitNewPosition`.
fn breaker_visit(cx: &mut Cx<'_>, actor: &mut Actor, pos: Pos) {
    let kind = actor.kind;
    // `!stateVisited.isRedstoneConductor`: a full block has nobody inside it.
    let solid = cx.host.block_collision(pos).iter().any(|b| *b == [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    if !solid {
        damage_entities(cx, actor, pos);
    }
    if !can_break(&*cx.host, kind, pos) {
        return;
    }
    actor.breaker.start(pos);
    actor.ctx.stall = true;
}

/// `SawMovementBehaviour.visitNewPosition`'s tail: a saw turned to face up or down lets go of its
/// block.
fn saw_visit(cx: &mut Cx<'_>, actor: &mut Actor) {
    let Some(facing) = actor.state.dir("facing") else { return };
    let turned = cx.rotated(facing.normal().map(f64::from));
    if super::actor::nearest(turned).axis() == create_kinetics::Axis::Y && actor.breaker.breaking.is_some() {
        actor.breaker.breaking = None;
        actor.ctx.stall = false;
    }
}

/// `cancelStall`: the crack goes and the structure is let go.
fn cancel_stall(cx: &mut Cx<'_>, actor: &mut Actor) {
    if let Some(at) = actor.breaker.cancel() {
        cx.host.set_break_progress(actor.id, at, -1);
    }
    actor.ctx.stall = false;
}

/// `BlockBreakingMovementBehaviour.destroyBlock`: broken with the drops into the structure's storage.
fn destroy(cx: &mut Cx<'_>, actor: &Actor, pos: Pos) {
    if actor.kind == Kind::Roller {
        super::roller_actor::destroy(cx, actor, pos);
        return;
    }
    if let Some(drops) = cx.host.destroy_block(pos, None, true) {
        for stack in drops {
            cx.collect_or_drop(&actor.ctx, stack);
        }
    }
}

/// `BlockBreakingMovementBehaviour.tick` and `tickBreaker`.
fn breaker_tick(cx: &mut Cx<'_>, actor: &mut Actor) {
    let mut speed = rules::breaking_speed(actor.ctx.animation_speed(), cx.carrier);
    if actor.kind == Kind::Roller {
        speed = rules::roller_breaking_speed(speed);
    }
    let kind = actor.kind;
    let step = {
        let host: &dyn Host = &*cx.host;
        actor.breaker.tick_breaker(
            &mut actor.ctx,
            speed,
            &|p| target_of(host, kind, p),
            &|p| rules::is_falling(&host.block(p)),
        )
    };
    match step {
        rules::Step::Idle => {}
        rules::Step::Cleared(pos) => cx.host.set_break_progress(actor.id, pos, -1),
        rules::Step::Cracking { pos, stage } => cx.host.set_break_progress(actor.id, pos, stage),
        rules::Step::Broken { cleared, pos, original } => {
            cx.host.set_break_progress(actor.id, cleared, -1);
            let broken = host_block(cx, pos);
            // `shouldDestroyStartBlock`: always, as the Saw's dynamic trees are another mod's.
            destroy(cx, actor, pos);
            on_block_broken(cx, actor, original, &broken);
        }
    }
    // The wait after a falling block, then the position is visited again.
    if let Some(last) = actor.breaker.tick_waiting(&mut actor.ctx) {
        if actor.kind == Kind::Roller {
            super::roller_actor::visit(cx, actor, last);
            return;
        }
        breaker_visit(cx, actor, last);
        if actor.kind == Kind::Saw {
            saw_visit(cx, actor);
        }
        if actor.kind == Kind::Plough {
            plough_till(cx, actor, last);
        }
    }
}

fn host_block(cx: &Cx<'_>, pos: Pos) -> String {
    cx.host.block(pos)
}

/// `onBlockBroken` of each breaker.
fn on_block_broken(cx: &mut Cx<'_>, actor: &mut Actor, pos: Pos, broken: &str) {
    if actor.kind == Kind::Roller {
        super::roller_actor::on_broken(cx, actor, pos, broken);
        return;
    }
    if actor.kind == Kind::Saw {
        // `SawMovementBehaviour.onBlockBroken`: leaves don't take a tree with them; otherwise the
        // tree the block held up falls, its drops into the storage or thrown at the saw's motion.
        if in_tag(&*cx.host, broken, "minecraft:leaves") {
            return;
        }
        let ctx = actor.ctx.clone();
        saw::cut_tree(cx.host, pos, broken, &mut |host, at, stack| {
            let rest = cx.store.insert(&*host, stack);
            if rest.count == 0 {
                return;
            }
            // `dropItemFromCutTree`: from the block's centre, moving with the structure the
            // further it was from the actor.
            let centre = util::centre(at);
            let distance = ctx.position.map_or(1.0, |p| {
                ((centre[0] - p[0]).powi(2) + (centre[1] - p[1]).powi(2) + (centre[2] - p[2]).powi(2)).sqrt()
            });
            let motion = ctx.relative_motion.map(|c| c * (distance / 20.0));
            util::spawn_item(host, centre, &rest, motion, 0);
        });
        return;
    }
    // `BlockBreakingMovementBehaviour.onBlockBroken`: sand and gravel come down, so wait for them.
    if rules::is_falling(broken) {
        actor.breaker.wait(&mut actor.ctx, FALLING_WAIT, pos);
    }
    if actor.kind == Kind::Plough && plough::drops_with_shovel(broken) {
        // `PloughMovementBehaviour.onBlockBroken`: snow's drops, rolled for an iron shovel.
        let shovel = ItemStack { item: "minecraft:iron_shovel".into(), count: 1, damage: 0, extra: None };
        let drops = cx.host.destroy_block(pos, Some(&shovel), true).unwrap_or_default();
        for stack in drops {
            cx.collect_or_drop(&actor.ctx, stack);
        }
    }
}

// --- hurting what is in the way -----------------------------------------------------------------------------------

/// `BlockBreakingMovementBehaviour.damageEntities` for the cell the actor enters.
pub(super) fn damage_entities(cx: &mut Cx<'_>, actor: &Actor, pos: Pos) {
    let damage_type = match actor.kind {
        Kind::Drill => Some("create:mechanical_drill"),
        Kind::Saw => Some("create:mechanical_saw"),
        Kind::Roller => Some("create:mechanical_roller"),
        _ => None,
    };
    // The Plough only throws (`throwsEntities`); the others both hurt and throw.
    let throws = matches!(actor.kind, Kind::Drill | Kind::Saw | Kind::Roller | Kind::Plough);
    let host = &mut *cx.host;
    let dimension = host.dimension();
    let [x, y, z] = pos.map(f64::from);
    let motion = actor.ctx.relative_motion;
    let speed = (motion[0] * motion[0] + motion[1] * motion[1] + motion[2] * motion[2]).sqrt();
    let damage = (6.0 * speed.powf(0.4) + 1.0).clamp(2.0, 10.0) as f32;
    for id in host.entities_in(dimension, [x - 1.0, y - 3.0, z - 1.0], [x + 2.0, y + 1.0, z + 2.0]) {
        let Some(entity) = host.entity(id) else { continue };
        let kind = host.entity_kind(entity).unwrap_or_default();
        if kind == "minecraft:item" || kind == "minecraft:marker" {
            continue;
        }
        let Some(bounds) = host.entity_bounding_box(entity) else { continue };
        if !(bounds[0][0] < x + 1.0
            && bounds[1][0] > x
            && bounds[0][1] < y + 1.0
            && bounds[1][1] > y
            && bounds[0][2] < z + 1.0
            && bounds[1][2] > z)
        {
            continue;
        }
        if let Some(damage_type) = damage_type {
            host.hurt_entity(dimension, id, damage, damage_type, None);
        }
        if throws && kind != "minecraft:player" {
            // `throwEntity`: the structure's motion and a lift of a quarter of it, at most 4 long.
            let m = actor.ctx.motion;
            let mut boost = [m[0], m[1] + speed_of(m) / 4.0, m[2]];
            let length = speed_of(boost);
            if length > 4.0 {
                boost = boost.map(|c| c - c / length * (length - 4.0));
            }
            let have = host.entity_velocity(entity).unwrap_or([0.0; 3]);
            host.entity_set_velocity(entity, [have[0] + boost[0], have[1] + boost[1], have[2] + boost[2]]);
        }
    }
}

fn speed_of(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

// --- the Plough's hoe ---------------------------------------------------------------------------------------------------

/// `PloughMovementBehaviour.visitNewPosition`: the diamond hoe is used on the block below.
fn plough_till(cx: &mut Cx<'_>, _actor: &Actor, pos: Pos) {
    let host = &mut *cx.host;
    let under = below(pos);
    let air_above = drill_rules::is_air(&host.block(pos));
    let block = host.block(under);
    let Some(tilled) = plough::till(&block, air_above) else { return };
    let new_state = State::new(tilled.block);
    // A tilled block keeps nothing of the old one; farmland starts dry (`moisture` 0).
    let new_state = if tilled.block == "minecraft:farmland" { new_state.with("moisture", "0") } else { new_state };
    if !engine::place_state(host, under, &new_state) {
        return;
    }
    host.play_sound("minecraft:item.hoe.till", util::centre(under), BLOCKS, 1.0, 1.0);
    if let Some(item) = tilled.drops {
        let stack = ItemStack { item: item.into(), count: 1, damage: 0, extra: None };
        util::spawn_item(host, util::centre(pos), &stack, [0.0; 3], 10);
    }
}

// --- the Harvester ---------------------------------------------------------------------------------------------------------

/// `HarvesterMovementBehaviour.visitNewPosition`.
fn harvest(cx: &mut Cx<'_>, actor: &mut Actor, pos: Pos) {
    let state = state_at(&*cx.host, pos);
    let air = state.is_air();
    let non_harvestable = in_tag(&*cx.host, &state.name, "create:non_harvestable");
    let plant = Plant {
        block: &state.name,
        age: state.get("age").and_then(|v| v.parse().ok()),
        collision_empty: collision_empty(&*cx.host, pos),
        leaves: in_tag(&*cx.host, &state.name, "minecraft:leaves"),
        cane_variant: in_tag(&*cx.host, &state.name, "create:sugar_cane_variants"),
    };
    let Reap::Cut { shears, cuttable, .. } = harvester::assess(DEFAULTS, &plant, air, non_harvestable) else {
        return;
    };
    let tool = shears.then(|| ItemStack { item: "minecraft:shears".into(), count: 1, damage: 0, extra: None });
    let drops = cx.host.destroy_block(pos, tool.as_ref(), true).unwrap_or_default();
    // `harvesterReplants`: one of the seed it drops stays in the ground.
    let seed = harvester::block_item(&state.name).to_owned();
    let mut seed_subtracted = cuttable;
    for mut stack in drops {
        if DEFAULTS.replants && !seed_subtracted && *stack.item == *seed && stack.count > 0 {
            stack.count -= 1;
            seed_subtracted = true;
        }
        if stack.count > 0 {
            cx.collect_or_drop(&actor.ctx, stack);
        }
    }
    if let Left::Aged(age) = harvester::cut_crop(DEFAULTS, &plant) {
        let mut cut = state.clone();
        cut.set("age", &age.to_string());
        engine::place_state(cx.host, pos, &cut);
    }
}

// --- the Deployer ---------------------------------------------------------------------------------------------------------------

const SLOT: &str = "Slot";

/// Reads the hand (slot 0 held, the rest the overflow) out of the block's saved data.
fn hand_of(data: &Compound) -> Items {
    let mut items: Items = vec![None; hand::SLOTS];
    if let Some(Tag::List(list)) = util::get(data, "Items") {
        for entry in list {
            let slot = match entry.get(SLOT) {
                Some(Tag::Byte(s)) => usize::from(*s as u8),
                Some(Tag::Int(s)) => usize::try_from(*s).unwrap_or(usize::MAX),
                _ => continue,
            };
            if slot < items.len() {
                items[slot] = util::stack_of(entry);
            }
        }
    }
    items
}

/// Writes the hand back into the data a Deployer's block has (`Items`, and `HeldItem` for clients).
fn write_hand(data: &mut Compound, items: &Items) {
    let list: Vec<Tag> = items
        .iter()
        .enumerate()
        .filter_map(|(slot, stack)| {
            let stack = stack.as_ref()?;
            let Tag::Compound(mut tag) = util::stack_tag(stack) else { return None };
            tag.insert(SLOT.into(), Tag::Byte(slot as i8));
            Some(Tag::Compound(tag))
        })
        .collect();
    data.insert("Items".into(), Tag::List(list));
    data.insert(
        "HeldItem".into(),
        items.first().cloned().flatten().map_or_else(|| Tag::Compound(Compound::new()), |s| util::stack_tag(&s)),
    );
}

fn hand_mut(actor: &mut Actor) -> &mut Hand {
    if !matches!(actor.extra, Extra::Deployer(_)) {
        actor.extra = Extra::Deployer(Box::new(Hand { items: hand_of(&actor.data), crack: None, timer: 0 }));
    }
    match &mut actor.extra {
        Extra::Deployer(hand) => hand,
        Extra::None | Extra::Contact(_) | Extra::Bell { .. } => unreachable!("the hand was just made"),
    }
}

fn mode_of(actor: &Actor) -> Mode {
    match util::get(&actor.data, "Mode") {
        Some(Tag::String(name)) => Mode::from_name(&name.to_string_lossy()).unwrap_or(Mode::Use),
        _ => Mode::Use,
    }
}

/// The Deployer's cell is two blocks in front of it, so a virtual Deployer at `cell - 2 * facing`
/// makes the stationary Deployer's code work on the visited cell.
fn virtual_deployer(cell: Pos, facing: Dir) -> Pos {
    let n = facing.normal();
    [cell[0] - 2 * n[0], cell[1] - 2 * n[1], cell[2] - 2 * n[2]]
}

/// `tryGrabbingItem`: an empty hand takes one item the filter passes out of the structure's storage.
fn grab(cx: &mut Cx<'_>, actor: &mut Actor) {
    let filter = chute_inv::filter_of(&actor.data);
    let hand = hand_mut(actor);
    if hand.items.first().is_some_and(Option::is_some) {
        return;
    }
    let host: &dyn Host = &*cx.host;
    let test = |stack: &ItemStack| filter.test(host, stack);
    let taken = cx.store.extract(host, &test, 1, false);
    if let Some(stack) = taken {
        hand.items[0] = Some(stack);
    }
}

/// `tryDisposeOfExcess`: everything but the held item that the filter passes goes back into the
/// structure's storage, or out into the world.
fn dispose_excess(cx: &mut Cx<'_>, actor: &mut Actor) {
    let ctx = actor.ctx.clone();
    let filter = chute_inv::filter_of(&actor.data);
    let hand = hand_mut(actor);
    for slot in 1..hand.items.len() {
        if let Some(stack) = hand.items[slot].take() {
            cx.collect_or_drop(&ctx, stack);
        }
    }
    // A held item the filter no longer passes is sent back too.
    let passes = hand.items.first().cloned().flatten().is_none_or(|held| filter.test(&*cx.host, &held));
    if !passes && let Some(stack) = hand.items[0].take() {
        cx.collect_or_drop(&ctx, stack);
    }
}

/// `DeployerHandler.activate` on the visited cell, as the stationary Deployer does it.
fn activate(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos) {
    let Some(facing) = actor.state.dir("facing") else { return };
    // The facing turned by the structure, to the nearest block direction.
    let turned = super::actor::nearest(cx.rotated(facing.normal().map(f64::from)));
    let mode = mode_of(actor);
    let at = virtual_deployer(cell, turned);
    let hand = hand_mut(actor);
    hand::activate(cx.host, at, turned, mode, &mut hand.items, &mut hand.crack);
}

/// `DeployerMovementBehaviour.visitNewPosition`.
fn deployer_visit(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos) {
    grab(cx, actor);
    let Some(facing) = actor.state.dir("facing") else { return };
    let turned = super::actor::nearest(cx.rotated(facing.normal().map(f64::from)));
    let mode = mode_of(actor);
    {
        let held = hand_mut(actor).items.first().cloned().flatten();
        if mode == Mode::Use && !should_activate(&*cx.host, cell, turned, held.as_ref()) {
            return;
        }
    }
    activate(cx, actor, cell);
    dispose_excess(cx, actor);
    actor.ctx.stall = hand_mut(actor).crack.is_some();
}

/// `DeployerHandler.shouldActivate`: not when the held item's block, or a bucket's fluid, is
/// already there.
fn should_activate(host: &dyn Host, cell: Pos, facing: Dir, held: Option<&ItemStack>) -> bool {
    let target = host.block(cell);
    let item = held.map(|stack| &*stack.item);
    let block = item.filter(|id| host.default_state(id).is_some());
    deployer_rules::should_activate(
        item,
        block,
        item.and_then(deployer_rules::bucket_fluid),
        &target,
        facing,
        false,
    )
}

/// `DeployerMovementBehaviour.tick`: a block being mined is punched again every 20 ticks.
fn deployer_tick(cx: &mut Cx<'_>, actor: &mut Actor) {
    if !actor.ctx.stall {
        return;
    }
    let mode = mode_of(actor);
    let hand = hand_mut(actor);
    let Some(crack) = hand.crack else {
        actor.ctx.stall = false;
        return;
    };
    if hand.timer < 20 {
        hand.timer += 1;
        return;
    }
    hand.timer = 0;
    let _ = mode;
    let cell = crack.pos;
    activate(cx, actor, cell);
    dispose_excess(cx, actor);
    actor.ctx.stall = hand_mut(actor).crack.is_some();
}

/// `DeployerMovementBehaviour.stopMoving`: the crack goes, and the hand goes back into the block's
/// data so the Deployer set down holds what it did.
fn deployer_stop(cx: &mut Cx<'_>, actor: &mut Actor) {
    let id = actor.id;
    let local = actor.local;
    let hand = hand_mut(actor);
    if let Some(crack) = hand.crack.take() {
        cx.host.set_break_progress(id, crack.pos, -1);
    }
    let items = hand.items.clone();
    actor.ctx.stall = false;
    let moving = cx.store.moving_mut();
    if let Some(block) = moving.blocks.iter_mut().find(|b| b.local == local) {
        write_hand(block.data.get_or_insert_with(Compound::new), &items);
    }
}

// --- the Funnels ---------------------------------------------------------------------------------------------------------------------

/// `FunnelMovementBehaviour.visitNewPosition`.
fn funnel(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos, brass: bool) {
    if actor.state.flag("extracting") {
        funnel_extract(cx, actor, cell, brass);
    } else {
        funnel_succ(cx, actor, cell);
    }
}

fn funnel_extract(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos, brass: bool) {
    let host: &dyn Host = &*cx.host;
    if !collision_empty(host, cell) {
        return;
    }
    let Some(at) = actor.ctx.position else { return };
    let facing = actor.state.dir("facing");
    // `entityPos = position`, a half block lower unless the funnel faces down.
    let drop_at = if facing == Some(Dir::Down) { at } else { [at[0], at[1] - f64::from(0.5f32), at[2]] };
    let drop_cell = drop_at.map(|c| c.floor() as i32);
    let dimension = host.dimension();
    let lo = drop_cell.map(f64::from);
    let hi = drop_cell.map(|c| f64::from(c) + 1.0);
    if !host.item_entities(dimension, lo, hi).is_empty() {
        return;
    }
    let filter = chute_inv::filter_of(&actor.data);
    let (count, up_to) = (filter.amount.count, filter.amount.up_to);
    let amount = if brass { count } else { 1 };
    let test = |stack: &ItemStack| filter.test(host, stack);
    let Some(stack) = cx.store.extract(host, &test, amount.max(1), !up_to || !brass) else { return };
    let spawned = util::spawn_item(cx.host, drop_at, &stack, [0.0; 3], 5);
    if spawned.is_some() {
        cx.host.play_sound("minecraft:entity.item.pickup", util::centre(cell), BLOCKS, 1.0 / 16.0, 0.1);
    }
}

fn funnel_succ(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos) {
    let dimension = cx.host.dimension();
    let lo = cell.map(f64::from);
    let hi = cell.map(|c| f64::from(c) + 1.0);
    let filter = chute_inv::filter_of(&actor.data);
    for item in cx.host.item_entities(dimension, lo, hi) {
        if !filter.test(&*cx.host, &item.stack) {
            continue;
        }
        let rest = cx.store.insert(&*cx.host, item.stack.clone());
        if rest.count == item.stack.count {
            continue;
        }
        // All of it in: the entity goes; some of it: the entity keeps the rest.
        cx.host.set_item_entity(dimension, item.id, rest);
    }
}
