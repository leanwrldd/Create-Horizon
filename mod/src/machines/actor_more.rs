//! The actors that need more than a block break or a pick-up: the Redstone Contact
//! (`ContactMovementBehaviour`), the Contraption Controls (which only decide what is disabled,
//! `actor.rs`), the bells (`BellMovementBehaviour`, `HauntedBellMovementBehaviour`), the campfires
//! (`CampfireMovementBehaviour`) and the dispenser and dropper (`DispenserMovementBehaviour`,
//! `DropperMovementBehaviour`). The Roller is `roller_actor.rs`; the rest are in `actor_tools.rs`. The
//! rules are `create_kinetics::rules::actor_misc` and `rules::redstone_contact`.
//!
//! Known differences from Create:
//! - **Bells:** the Haunted Bell's soul pulse (`HauntedBellPulser.sendPulse`, a client effect) isn't drawn;
//!   it still rings and recharges. A bell on a train's carriage rings (there are no trains).
//! - **Campfires:** Create spawns the smoke on the client (`isClientSide`, `addAlwaysVisibleParticle`); here the
//!   server sends the particles to everyone near, with the same positions and the 0.07 rise as the speed of a
//!   `count: 0` packet (not played: whether the new API reads it so).
//! - **Dispensers:** the dispense sound and smoke (`levelEvent` 1000, 1001 and 2000) are a `play_sound` and a
//!   few smoke particles. Behaviours ported: the default (`DefaultMountedDispenseBehavior`, also the
//!   Dropper's), TNT and spawn eggs. Not reached (they dispense as plain items): the vanilla behaviours Create
//!   wraps (`DispenserBehaviorConverter`: arrows, eggs, snowballs, boats, armour, shears, bone meal, flint and
//!   steel, minecarts and the rest), fire charges, fireworks, buckets, glass bottles and the potions (Horizon
//!   gives a mod no way to throw, shoot or use them). A primed TNT starts with the vanilla fuse and has the
//!   structure's motion only (not the small hop `PrimedTnt` gives itself). A spawn egg's mob is the egg's name
//!   (`cow_spawn_egg` makes a `cow`). With a container in front the item goes into it; a full container is
//!   not told from none, so the item is thrown out then (Create leaves it in the dispenser).

use create_kinetics::rules::actor::Kind;
use create_kinetics::rules::actor_misc::{self as misc, BellKind, Rng};
use create_kinetics::rules::redstone_contact as rules;
use create_kinetics::{Dir, Pos};
use crate::compat::{Compound, Hooks, Host, ItemStack, ParticleOptions, ParticleType, RandomSource, Tag};

use super::actor::{Actor, Cx};
use super::actor_tools::Extra;
use super::diodes_signal as signal;
use super::redstone_contact as contact;
use crate::util;

struct Shared;

impl Hooks for Shared {}

/// A module of shared code: no hooks of its own.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

fn last_of(actor: &Actor) -> Option<Pos> {
    match actor.extra {
        Extra::Contact(last) => last,
        _ => None,
    }
}

/// The level's random as the rules' dice.
struct Dice<'a>(&'a mut dyn Host);

impl Rng for Dice<'_> {
    fn unit(&mut self) -> f64 {
        let dimension = self.0.dimension();
        self.0.level_random(dimension).map_or(0.5, |random| f64::from(random.next_float()))
    }

    fn int(&mut self, bound: i32) -> i32 {
        let dimension = self.0.dimension();
        self.0.level_random(dimension).and_then(|random| random.next_int_bound(bound).ok()).unwrap_or(0)
    }
}

fn play(host: &mut dyn Host, sound: misc::Sound, at: [f64; 3]) {
    // A Create entry plays all its layers (`SoundEntry.play`); a vanilla id plays as it is.
    match sound.id.strip_prefix("create:") {
        Some(name) => util::create_sound(host, name, at, sound.volume, sound.pitch),
        None => {
            host.play_sound(sound.id, at, BLOCKS, sound.volume, sound.pitch);
        }
    }
}

// --- the hooks --------------------------------------------------------------------------------------------

/// `visitNewPosition`.
pub(super) fn visit(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos) {
    match actor.kind {
        Kind::Contact => contact_visit(cx, actor, cell),
        Kind::Bell(BellKind::Haunted) => haunted_visit(cx, actor),
        Kind::Dispenser { dropper } => dispense_visit(cx, actor, cell, dropper),
        _ => {}
    }
}

/// `tick`.
pub(super) fn tick(cx: &mut Cx<'_>, actor: &mut Actor) {
    match actor.kind {
        Kind::Bell(BellKind::Haunted) => {
            let recharge = recharge_of(actor);
            actor.extra = Extra::Bell { moved: false, recharge: Some(misc::haunted_tick(recharge)) };
        }
        Kind::Bell(kind) => {
            let moved = matches!(actor.extra, Extra::Bell { moved: true, .. });
            let (moved, rings) = misc::bell_tick(moved, cx.elevator_arrived);
            actor.extra = Extra::Bell { moved, recharge: None };
            if rings {
                ring(cx, actor, kind);
            }
        }
        Kind::Campfire => campfire_tick(cx, actor),
        _ => {}
    }
}

/// `onSpeedChanged`, called when the actor's motion differs from the last tick's.
pub(super) fn speed_changed(cx: &mut Cx<'_>, actor: &mut Actor, old: [f64; 3]) {
    let Kind::Bell(kind) = actor.kind else { return };
    // `BellMovementBehaviour.onSpeedChanged`: not on an elevator.
    if cx.elevator_arrived.is_some() {
        return;
    }
    let moving = actor.ctx.relative_motion.iter().any(|c| *c != 0.0);
    if misc::rings_on_speed_change(old, actor.ctx.motion, moving, actor.ctx.first_movement) {
        ring(cx, actor, kind);
    }
}

/// `stopMoving`.
pub(super) fn stop(cx: &mut Cx<'_>, actor: &mut Actor) {
    match actor.kind {
        // `ContactMovementBehaviour.stopMoving`: `deactivateLastVisitedContact`.
        Kind::Contact => {
            if let Some(old) = last_of(actor) {
                contact::release(signal::host_level(&*cx.host), old);
            }
            actor.extra = Extra::None;
        }
        // `BellMovementBehaviour.stopMoving`: `position != null && isActive`.
        Kind::Bell(kind) => {
            if actor.ctx.position.is_some() && !actor.ctx.disabled {
                ring(cx, actor, kind);
            }
            if kind == BellKind::Haunted {
                // `writeExtraData`.
                let recharge = recharge_of(actor);
                let local = actor.local;
                if let Some(block) = cx.store.moving_mut().blocks.iter_mut().find(|b| b.local == local) {
                    block.data.get_or_insert_with(Compound::new).insert("Recharge".into(), Tag::Int(recharge));
                }
                actor.data.insert("Recharge".into(), Tag::Int(recharge));
            }
            actor.extra = Extra::None;
        }
        _ => {}
    }
}

// --- the Redstone Contact ---------------------------------------------------------------------------------

/// `ContactMovementBehaviour.visitNewPosition`.
fn contact_visit(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos) {
    let Some(facing) = actor.state.dir("facing") else { return };
    let own = super::actor::nearest(cx.rotated(facing.normal().map(f64::from)));
    let visited = {
        let host: &dyn Host = &*cx.host;
        let block = host.block(cell);
        let is_contact = block == rules::ID || block == rules::ELEVATOR_ID;
        if is_contact { util::facing(host, cell, "facing").map(|f| (block == rules::ID, f)) } else { None }
    };
    let level = signal::host_level(&*cx.host);
    let result = rules::visit(actor.ctx.first_movement, last_of(actor), cell, own, visited);
    if let Some(old) = result.release {
        contact::release(level, old);
    }
    if let Some(new) = result.power {
        contact::hold(level, new);
    }
    actor.extra = Extra::Contact(result.last);
}

// --- bells ------------------------------------------------------------------------------------------------

/// `BellMovementBehaviour.playSound`: at the block position the actor is at.
fn ring(cx: &mut Cx<'_>, actor: &Actor, kind: BellKind) {
    let Some(position) = actor.ctx.position else { return };
    let cell = create_kinetics::rules::actor::floor(position);
    play(cx.host, misc::bell_sound(kind), util::centre(cell));
}

/// `HauntedBellMovementBehaviour.getRecharge`: the saved `Recharge` until the behaviour has its own.
fn recharge_of(actor: &Actor) -> i32 {
    match actor.extra {
        Extra::Bell { recharge: Some(recharge), .. } => recharge,
        _ => crate::int(&actor.data, "Recharge"),
    }
}

/// `HauntedBellMovementBehaviour.visitNewPosition`: when recharged, it pulses (not drawn), recharges and rings.
fn haunted_visit(cx: &mut Cx<'_>, actor: &mut Actor) {
    if let Some(next) = misc::haunted_visit(recharge_of(actor)) {
        actor.extra = Extra::Bell { moved: false, recharge: Some(next) };
        ring(cx, actor, BellKind::Haunted);
    }
}

// --- campfires --------------------------------------------------------------------------------------------

/// `CampfireMovementBehaviour.tick`.
fn campfire_tick(cx: &mut Cx<'_>, actor: &mut Actor) {
    let Some(position) = actor.ctx.position else { return };
    let puffs = misc::campfire_smoke(
        actor.state.flag("lit"),
        actor.ctx.disabled,
        actor.state.flag("signal_fire"),
        position,
        &mut Dice(cx.host),
    );
    for puff in puffs {
        let ty = if puff.signal { ParticleType::CampfireSignalSmoke } else { ParticleType::CampfireCosySmoke };
        // A `count` of 0 sends the spread as the velocity and the speed as its scale.
        cx.host.send_particles(&ParticleOptions::simple(ty), None, puff.at, misc::SMOKE_MOTION, 1.0, 0);
    }
}

// --- dispensers -------------------------------------------------------------------------------------------

fn slot_index(entry: &Tag) -> Option<usize> {
    match entry.get("Slot") {
        Some(Tag::Byte(slot)) => Some(usize::from(*slot as u8)),
        Some(Tag::Int(slot)) => usize::try_from(*slot).ok(),
        _ => None,
    }
}

/// The dispenser's own storage (`DispenserMountedStorage`, a `SimpleMountedStorage` of 9 slots): the block's
/// saved `Items`.
fn read_slots(data: Option<&Compound>) -> Vec<Option<ItemStack>> {
    let mut slots: Vec<Option<ItemStack>> = vec![None; misc::DISPENSER_SLOTS];
    if let Some(Tag::List(list)) = data.and_then(|d| util::get(d, "Items")) {
        for entry in list {
            let (Some(index), Some(stack)) = (slot_index(entry), util::stack_of(entry)) else { continue };
            if index < slots.len() && stack.count > 0 {
                slots[index] = Some(stack);
            }
        }
    }
    slots
}

fn write_slots(data: &mut Compound, slots: &[Option<ItemStack>]) {
    let mut entries = Vec::new();
    for (index, stack) in slots.iter().enumerate() {
        let Some(stack) = stack.as_ref().filter(|s| s.count > 0) else { continue };
        let Tag::Compound(mut tag) = util::stack_tag(stack) else { continue };
        tag.insert("Slot".into(), Tag::Byte(index as i8));
        entries.push(Tag::Compound(tag));
    }
    data.insert("Items".into(), Tag::List(entries));
}

fn write_back(cx: &mut Cx<'_>, local: Pos, slots: &[Option<ItemStack>]) {
    if let Some(block) = cx.store.moving_mut().blocks.iter_mut().find(|b| b.local == local) {
        write_slots(block.data.get_or_insert_with(Compound::new), slots);
    }
}

/// `ItemStack.isSameItemSameComponents`.
fn same_item(a: &ItemStack, b: &ItemStack) -> bool {
    a.item == b.item && a.damage == b.damage && a.extra == b.extra
}

/// `DropperMovementBehaviour.visitNewPosition`.
fn dispense_visit(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos, dropper: bool) {
    let Some(facing) = actor.state.dir("facing") else { return };
    let local = actor.local;
    let own = cx.store.moving_mut().blocks.iter().find(|b| b.local == local).and_then(|b| b.data.clone());
    let mut slots = read_slots(own.as_ref());
    // `getSlot`: the filled slots, the nearly empty ones topped off from the whole structure.
    let mut filled = Vec::new();
    for index in 0..slots.len() {
        let Some(stack) = slots[index].clone() else { continue };
        let max = i32::try_from(cx.host.max_stack_size(&stack.item).unwrap_or(64)).unwrap_or(64);
        if let Some(wanted) = misc::top_off_wanted(i32::try_from(stack.count).unwrap_or(i32::MAX), max) {
            let like = stack.clone();
            let Some(extra) = cx.store.extract(&*cx.host, &|other| same_item(&like, other), wanted, false) else {
                continue;
            };
            let mut topped = stack;
            topped.count += extra.count;
            slots[index] = Some(topped);
        }
        filled.push(index);
    }
    let Some(index) = misc::pick_slot(&filled, &mut Dice(cx.host)) else {
        write_back(cx, local, &slots);
        // `failDispense`.
        play(cx.host, misc::FAIL_SOUND, util::centre(cell));
        return;
    };
    let normal = {
        let n = cx.rotated(facing.normal().map(f64::from));
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if length < 1.0e-5 { [0.0; 3] } else { n.map(|c| c / length) }
    };
    // The behaviours take a copy and answer what is left.
    let Some(stack) = slots[index].clone() else { return };
    let rest = dispense(cx, stack, cell, normal, actor.ctx.motion, dropper);
    slots[index] = Some(rest).filter(|s| s.count > 0);
    write_back(cx, local, &slots);
}

fn motion_tag(motion: [f64; 3]) -> Tag {
    Tag::List(motion.iter().map(|m| Tag::Double(m.to_bits())).collect())
}

/// `DefaultMountedDispenseBehavior.dispense` and the behaviours Create registers that are ported here: the
/// stack that is left.
fn dispense(cx: &mut Cx<'_>, mut stack: ItemStack, cell: Pos, normal: [f64; 3], motion: [f64; 3], dropper: bool) -> ItemStack {
    let dir: Dir = misc::closest(normal);
    let step = dir.normal();
    // A container in front takes one item as a hopper would (`HopperBlockEntity.addItem`).
    let target = [cell[0] + step[0], cell[1] + step[1], cell[2] + step[2]];
    let mut one = stack.clone();
    one.count = 1;
    let rest = cx.host.insert_item(target, util::direction(dir.opposite()), one);
    if rest.is_none_or(|r| r.count == 0) {
        stack.count = stack.count.saturating_sub(1);
        return stack;
    }
    let centre = util::centre(cell);
    let item = stack.item.to_string();
    let dimension = cx.host.dimension().to_owned();
    if !dropper && item == "minecraft:tnt" {
        // `AllMountedDispenseItemBehaviors.TNT`.
        let at = misc::tnt_origin(cell, normal);
        let mut nbt = Compound::new();
        nbt.insert("Motion".into(), motion_tag(motion));
        cx.host.summon(&dimension, "minecraft:tnt", at, nbt);
        play(cx.host, misc::TNT_SOUND, at);
    } else if let Some(mob) = misc::egg_entity(&item).filter(|_| !dropper) {
        // `AllMountedDispenseItemBehaviors.SPAWN_EGG`: the mob appears at the bottom centre of the block.
        let egg = misc::spawn_egg_cell(cell, normal);
        let at = [f64::from(egg[0]) + 0.5, f64::from(egg[1]), f64::from(egg[2]) + 0.5];
        let mut nbt = Compound::new();
        nbt.insert("Motion".into(), motion_tag(misc::spawn_egg_motion(motion)));
        cx.host.summon(&dimension, &mob, at, nbt);
    } else {
        // `execute`: one item, thrown out in front.
        let mut thrown = stack.clone();
        thrown.count = 1;
        let (at, speed) = misc::spawn_item(cell, normal, motion, &mut Dice(cx.host));
        util::spawn_item(cx.host, at, &thrown, speed, 0);
    }
    stack.count = stack.count.saturating_sub(1);
    // `playSound` (levelEvent 1000) and `playAnimation` (2000, smoke towards the facing).
    play(cx.host, misc::DISPENSE_SOUND, centre);
    let front = [0, 1, 2].map(|a| centre[a] + f64::from(step[a]) * 0.6);
    cx.host.send_particles(&ParticleOptions::simple(ParticleType::Smoke), None, front, [0.1, 0.1, 0.1], 0.02, 10);
    stack
}
