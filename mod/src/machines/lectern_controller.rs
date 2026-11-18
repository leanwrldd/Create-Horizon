//! The Lectern Controller (`LecternControllerBlock`, `LecternControllerBlockEntity`, `LecternControllerRenderer`): a
//! Linked Controller put on a lectern that has no book. A player standing next to it uses it until they walk away,
//! as with the controller in hand: the keys they hold keep its frequencies transmitting. The session rules, the
//! range and the drop's place are `create_kinetics::rules::lectern_controller` (its header lists the known
//! differences from Create); the controller's keys and messages are `linked_controller.rs`, which this extends.
//!
//! Using it (as in Create):
//! - right-click a lectern with no book holding a Linked Controller: the controller goes on it (Creative keeps it);
//! - right-click the Lectern Controller from close by (within 0.4 of your reach): you use it until you leave, put
//!   the keys away by walking off, or break it;
//! - sneak + right-click: the controller comes off as an item and the block is a plain lectern again.
//!
//! The block entity (`create:lectern_controller`) saves `ControllerData` (the controller's frequency items) and
//! `User` (the player's UUID as four ints). Which players use a lectern is kept in memory (Create's
//! `IsUsingLecternController` mark), with the position of their eyes as their own tick last saw it, so the block
//! entity's tick can end a session whose player went. The player's client is told when to start and stop (the
//! block entity's `prevUser` watching of Create's `tick`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use create_kinetics::rules::lectern_controller::{self as rules, Session, User};
use create_kinetics::rules::linked_controller as controller_rules;
use create_kinetics::rules::redstone_link::horizontal_angle;
use crate::compat::{
    AnalogOutputEvent, BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, Compound, DestroyBlockEvent, DisplayContext, Flow,
    HasAnalogOutputEvent, Hooks, Host, ItemStack, LevelHost, LevelTickPostEvent, LootDropsEvent,
    NativeInventoryKind, NetFromServerEvent, PlayerTickEvent, RightClickBlockEvent, Tag, UseItemOnEvent,
};

use super::diodes_signal as signal;
use super::linked_controller as controller;
use crate::util;

pub const ID: &str = rules::BLOCK;

const CONTROLLER_DATA: &str = "ControllerData";
const USER: &str = "User";
/// The channel the server tells a player's client when their lectern session starts (1 and the position) or ends (0).
const CHANNEL: &str = "create:lectern_controller";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// Ticks a user's own tick may be missing before the lectern lets them go.
const SEEN_MAX_AGE: i64 = 5;
/// `Player.getEyeHeight`.
const EYE: f64 = 1.62;
const EYE_SNEAKING: f64 = 1.27;

struct Lectern;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Lectern)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- Memory ---------------------------------------------------------------------------------------------------

/// A player who uses a lectern, with where their eyes were when their own tick last ran.
#[derive(Clone, Copy)]
struct Seen {
    tick: i64,
    eye: [f64; 3],
}

/// `IsUsingLecternController`: the players using a lectern.
static USING: LazyLock<Mutex<HashMap<u128, Seen>>> = LazyLock::new(Default::default);
/// `deactivatedThisTick`, by dimension and position.
static DEACTIVATED: LazyLock<Mutex<HashSet<(&'static str, [i32; 3])>>> = LazyLock::new(Default::default);
/// The Lectern Controllers whose block entities are ticking, by level and position, with the tick they were last
/// seen (a comparator's hook has only the level's blocks).
static TICKING: LazyLock<Mutex<HashMap<(u8, [i32; 3]), i64>>> = LazyLock::new(Default::default);
/// The latest game time a block entity saw.
static NOW: AtomicI64 = AtomicI64::new(0);
/// A controller to put on a lectern, queued from the player's hook.
static PUTS: LazyLock<Mutex<Vec<Put>>> = LazyLock::new(Default::default);

struct Put {
    uuid: u128,
    dimension: &'static str,
    pos: [i32; 3],
    stack: ItemStack,
    slot: usize,
    creative: bool,
}

// ---- The data ----------------------------------------------------------------------------------------------------

/// `CompoundTag.putUUID`: four ints, most significant first.
fn uuid_tag(uuid: u128) -> Tag {
    Tag::IntArray((0..4).map(|i| (uuid >> (96 - 32 * i)) as u32 as i32).collect())
}

fn uuid_of(tag: &Tag) -> Option<u128> {
    match tag {
        Tag::IntArray(ints) if ints.len() == 4 => Some(ints.iter().fold(0u128, |acc, i| acc << 32 | u128::from(*i as u32))),
        _ => None,
    }
}

/// The player using the lectern, from its saved data.
fn user_of(nbt: &Compound) -> Option<u128> {
    util::get(nbt, USER).and_then(uuid_of)
}

fn set_user(nbt: &mut Compound, user: Option<u128>) {
    match user {
        Some(uuid) => {
            nbt.insert(USER.into(), uuid_tag(uuid));
        }
        None => {
            util::remove(nbt, USER);
        }
    }
}

/// `LecternControllerBlockEntity.isUsedBy`.
pub(super) fn is_used_by(nbt: &Compound, player: u128) -> bool {
    user_of(nbt) == Some(player)
}

/// The frequency items a lectern holds, for the controller's keys.
pub(super) fn controller_items(nbt: &Compound) -> Option<Vec<Option<ItemStack>>> {
    match util::get(nbt, CONTROLLER_DATA) {
        Some(Tag::Compound(data)) => Some(controller::items_of_tag(data)),
        _ => None,
    }
}

/// `createLinkedController`: the item with its frequencies.
fn controller_stack(host: &dyn Host, data: &Compound) -> ItemStack {
    let mut stack = ItemStack { item: controller_rules::CONTROLLER.into(), count: 1, damage: 0, extra: None };
    let _ = host.set_component(&mut stack, controller_rules::COMPONENT, Tag::Compound(data.clone()));
    stack
}

fn eye_of(position: [f64; 3], sneaking: bool) -> [f64; 3] {
    [position[0], position[1] + if sneaking { EYE_SNEAKING } else { EYE }, position[2]]
}

// ---- A session ------------------------------------------------------------------------------------------------

/// Tells a player's client its lectern session started at `pos` or ended.
fn tell_client(host: &mut dyn Host, uuid: u128, start: Option<[i32; 3]>) {
    let mut data = Vec::new();
    match start {
        Some(pos) => {
            data.push(1);
            for v in pos {
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
        None => data.push(0),
    }
    let _ = host.net_send_to_player(uuid, CHANNEL, &data);
}

/// `tryStartUsing`, for the player acting now.
fn try_start(host: &mut dyn Host, uuid: u128, pos: [i32; 3], eye: [f64; 3]) -> bool {
    let dimension = host.dimension();
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    let mut session = Session {
        user: user_of(&entity.nbt),
        deactivated_this_tick: lock(&DEACTIVATED).contains(&(dimension, pos)),
    };
    let using = lock(&USING).contains_key(&uuid);
    if !session.try_start(uuid, rules::in_range(eye, pos), using) {
        return false;
    }
    host.change_mod_block_entity(pos, &mut |e| set_user(&mut e.nbt, session.user));
    host.block_entity_sync(pos);
    let tick = util::game_time(&*host);
    lock(&USING).insert(uuid, Seen { tick, eye });
    tell_client(host, uuid, Some(pos));
    true
}

/// `tryStopUsing` (`LinkedControllerStopLecternPacket`): the player is done with the lectern.
pub(super) fn try_stop(host: &mut dyn Host, uuid: u128, pos: [i32; 3]) {
    if host.block(pos) != ID {
        return;
    }
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let mut session = Session { user: user_of(&entity.nbt), deactivated_this_tick: false };
    if session.try_stop(uuid) {
        host.change_mod_block_entity(pos, &mut |e| set_user(&mut e.nbt, None));
        host.block_entity_sync(pos);
        lock(&DEACTIVATED).insert((host.dimension(), pos));
        lock(&USING).remove(&uuid);
        tell_client(host, uuid, None);
    }
}

/// `LecternControllerBlockEntity.dropController`: the session ends, and the controller falls in front of the lectern.
fn drop_controller(host: &mut dyn Host, pos: [i32; 3]) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let Some(Tag::Compound(data)) = util::get(&entity.nbt, CONTROLLER_DATA).cloned() else {
        return;
    };
    if let Some(user) = user_of(&entity.nbt) {
        lock(&USING).remove(&user);
        tell_client(host, user, None);
    }
    host.change_mod_block_entity(pos, &mut |e| {
        set_user(&mut e.nbt, None);
        util::remove(&mut e.nbt, CONTROLLER_DATA);
    });
    let facing = host.block_property(pos, "facing").unwrap_or_default();
    let stack = controller_stack(&*host, &data);
    util::spawn_item(host, rules::drop_at(pos, &facing), &stack, [0.0; 3], 10);
}

/// `LecternControllerBlock.replaceWithLectern`.
fn replace_with_lectern(host: &mut dyn Host, pos: [i32; 3]) {
    let facing = host.block_property(pos, "facing").unwrap_or_else(|| "north".to_owned());
    let powered = host.block_property(pos, "powered").as_deref() == Some("true");
    drop_controller(host, pos);
    let dimension = host.dimension();
    let Some(state) = host.default_state(rules::LECTERN) else {
        return;
    };
    host.play_sound("create:controller_take", util::centre(pos), BLOCKS, 1.0, 1.0);
    if host.set_block(dimension, pos, state, util::UPDATE_ALL) {
        host.set_block_property(pos, "facing", &facing);
        host.set_block_property(pos, "powered", if powered { "true" } else { "false" });
    }
}

/// `LecternControllerBlock.replaceLectern`: the controller goes on the lectern, taking its `facing` and `powered`.
fn put(host: &mut dyn Host, job: &Put) {
    if host.held_stack(job.uuid).as_ref() != Some(&job.stack) {
        return;
    }
    let dimension = host.dimension();
    if !rules::takes_controller(&host.block(job.pos), host.block_property(job.pos, "has_book").as_deref() == Some("true")) {
        return;
    }
    let facing = host.block_property(job.pos, "facing").unwrap_or_else(|| "north".to_owned());
    let powered = host.block_property(job.pos, "powered").as_deref() == Some("true");
    if !job.creative {
        // `stack.split(1)`.
        let mut rest = job.stack.clone();
        rest.count -= 1;
        let rest = (rest.count > 0).then_some(rest);
        if host.set_inventory_slot_typed(job.uuid, NativeInventoryKind::Player, job.slot, Some(&job.stack), rest).is_err() {
            return;
        }
    }
    let Some(state) = host.default_state(ID) else {
        return;
    };
    if !host.set_block(dimension, job.pos, state, util::UPDATE_ALL) {
        return;
    }
    host.set_block_property(job.pos, "facing", &facing);
    host.set_block_property(job.pos, "powered", if powered { "true" } else { "false" });
    let data = match job.stack.data().and_then(|d| d.other(controller_rules::COMPONENT)) {
        Some(Tag::Compound(tag)) => tag.clone(),
        _ => Compound::new(),
    };
    host.change_mod_block_entity(job.pos, &mut |e| {
        e.nbt.insert(CONTROLLER_DATA.into(), Tag::Compound(data.clone()));
    });
    host.block_entity_sync(job.pos);
    host.play_sound("create:controller_put", util::centre(job.pos), BLOCKS, 1.0, 1.0);
}

fn is_lectern_state(level: &dyn LevelHost, state: crate::compat::BlockStateId) -> bool {
    level.state_with_property(state, "has_book", "true").is_some()
        && level.state_with_property(state, "powered", "true").is_some()
        && level.state_with_property(state, "facing", "north").is_some()
}

impl Hooks for Lectern {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `LecternControllerBlockEntity.tick` (server): a user who is gone, out of range or no longer marked is let go.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let now = util::game_time(&*host);
        NOW.store(now, Ordering::Relaxed);
        let dimension = host.dimension();
        lock(&TICKING).insert((signal::host_level(&*host), event.pos), now);
        lock(&DEACTIVATED).remove(&(dimension, event.pos));
        let mut session = Session { user: user_of(&event.entity.nbt), deactivated_this_tick: false };
        let Some(user) = session.user else {
            return;
        };
        let status = match lock(&USING).get(&user) {
            Some(seen) if now - seen.tick <= SEEN_MAX_AGE => {
                User::Present { in_range: rules::in_range(seen.eye, event.pos), using: true }
            }
            _ => User::Gone,
        };
        if let Some(who) = session.tick(status) {
            set_user(&mut event.entity.nbt, None);
            lock(&DEACTIVATED).insert((dimension, event.pos));
            lock(&USING).remove(&who);
            tell_client(host, who, None);
            host.block_entity_sync(event.pos);
        }
    }

    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block == ID {
            *event.bounds = [0.0, 0.0, 0.0, 1.0, 2.0, 1.0];
        }
    }

    /// `LecternControllerRenderer`: the controller lies on the slope, tilted 22 degrees. The pose follows Create's
    /// (`translate(0.5, 1.45, 0.5)`, turn to the facing, `translate(0.28, 0, 0)`, `rotateZ(-22)`); it has not been
    /// looked at in game.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let facing = match host.block_state_property(event.state, "facing") {
            Some("east") => [1, 0, 0],
            Some("west") => [-1, 0, 0],
            Some("south") => [0, 0, 1],
            _ => [0, 0, -1],
        };
        let stack = ItemStack { item: controller_rules::CONTROLLER.into(), count: 1, damage: 0, extra: None };
        let draw = &mut *event.draw;
        draw.push();
        draw.translate([0.5, 1.45, 0.5]);
        draw.rotate_about([0.0, 1.0, 0.0], (horizontal_angle(facing) - 90.0) as f32, [0.0; 3]);
        draw.translate([0.28, 0.0, 0.0]);
        draw.rotate_about([0.0, 0.0, 1.0], -22.0, [0.0; 3]);
        host.draw_item(draw, &stack, DisplayContext::Fixed, None, 0);
        draw.pop();
        draw.keep();
    }

    /// `LecternControllerBlock.useItemOn`: a click from close by uses it, sneaking takes the controller off.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        if host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let eye = eye_of(player.position, player.sneaking);
        if !player.sneaking && !rules::in_range(eye, event.pos) {
            return Flow::Continue;
        }
        // Heard once for each hand.
        if !util::first_click(host, player.uuid, event.pos) {
            return Flow::Cancel;
        }
        if player.sneaking {
            replace_with_lectern(host, event.pos);
        } else {
            try_start(host, player.uuid, event.pos, eye);
        }
        Flow::Cancel
    }

    /// `LinkedControllerItem.onItemUseFirst`: a controller on a lectern with no book. On the Lectern Controller itself
    /// it does nothing (`PASS`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != controller_rules::CONTROLLER {
            return Flow::Continue;
        }
        let block = host.block(event.pos);
        if block == ID {
            return Flow::Cancel;
        }
        let has_book = host.block_property(event.pos, "has_book").as_deref() == Some("true");
        if !rules::takes_controller(&block, has_book) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if host.held_stack(player.uuid).as_ref() != Some(event.stack) {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, event.pos) {
            return Flow::Cancel;
        }
        // A player's own hook can't change their stack: it is done on the next tick.
        lock(&PUTS).push(Put {
            uuid: player.uuid,
            dimension: host.dimension(),
            pos: event.pos,
            stack: event.stack.clone(),
            slot: usize::from(player.selected_slot),
            creative: player.game_mode == util::CREATIVE,
        });
        Flow::Cancel
    }

    /// `LecternControllerBlock.onRemove`: breaking it drops the controller.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        if host.block(event.pos) == ID {
            drop_controller(host, event.pos);
        }
        Flow::Continue
    }

    /// What breaks it some other way (an explosion, a piston) still drops the controller with the lectern.
    fn loot_drops(&self, host: &mut dyn Host, event: &mut LootDropsEvent<'_>) {
        let Some((pos, _)) = event.context.block else {
            return;
        };
        if !event.drops.iter().any(|drop| &*drop.item == rules::LECTERN) {
            return;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return;
        };
        if entity.id != ID {
            return;
        }
        if let Some(Tag::Compound(data)) = util::get(&entity.nbt, CONTROLLER_DATA) {
            let stack = controller_stack(&*host, data);
            event.drops.push(stack);
        }
    }

    /// The user's own tick: where their eyes are, for the block entity's range check.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let tick = util::game_time(&*host);
        if let Some(seen) = lock(&USING).get_mut(&player.uuid) {
            *seen = Seen { tick, eye: eye_of(player.position, player.sneaking) };
        }
    }

    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        let dimension = host.dimension();
        let (mine, other): (Vec<Put>, Vec<Put>) = std::mem::take(&mut *lock(&PUTS))
            .into_iter()
            .partition(|job| job.dimension == dimension);
        lock(&PUTS).extend(other);
        for job in mine {
            put(host, &job);
        }
    }

    /// The client half: the server says the player now uses a lectern, or no longer does.
    fn net_from_server(&self, host: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        if event.channel != CHANNEL {
            return;
        }
        match event.data {
            [1, rest @ ..] if rest.len() == 12 => {
                let int = |i: usize| i32::from_le_bytes([rest[i], rest[i + 1], rest[i + 2], rest[i + 3]]);
                controller::lectern_on([int(0), int(4), int(8)]);
            }
            [0] => controller::lectern_off(host),
            _ => {}
        }
    }

    /// `LecternControllerBlock.getAnalogOutputSignal` always gives 15; a lectern is a comparator source already.
    fn has_analog_output(&self, level: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        if is_lectern_state(level, event.state) {
            event.output = true;
        }
    }

    fn analog_output(&self, level: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        if !is_lectern_state(level, event.state) {
            return;
        }
        let at = [event.pos.x, event.pos.y, event.pos.z];
        let seen = lock(&TICKING).get(&(signal::level_index(event.dimension), at)).copied();
        if seen.is_some_and(|tick| NOW.load(Ordering::Relaxed) - tick <= 3) {
            event.output = rules::ANALOG_OUTPUT;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uuid_round_trips_through_four_ints() {
        let uuid: u128 = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;
        let tag = uuid_tag(uuid);
        let Tag::IntArray(ints) = &tag else { panic!("not ints") };
        assert_eq!(ints.len(), 4);
        assert_eq!(ints[0], 0x0123_4567);
        assert_eq!(uuid_of(&tag), Some(uuid));
        assert_eq!(uuid_of(&Tag::Int(3)), None);
    }

    #[test]
    fn the_user_is_saved_and_cleared() {
        let mut nbt = Compound::new();
        assert_eq!(user_of(&nbt), None);
        set_user(&mut nbt, Some(42));
        assert_eq!(user_of(&nbt), Some(42));
        set_user(&mut nbt, None);
        assert_eq!(user_of(&nbt), None);
    }

    #[test]
    fn the_eye_follows_the_pose() {
        assert!((eye_of([1.0, 64.0, 2.0], false)[1] - 65.62).abs() < 1e-9);
        assert!((eye_of([1.0, 64.0, 2.0], true)[1] - 65.27).abs() < 1e-9);
    }
}
