//! The Placard (`PlacardBlock`, `PlacardBlockEntity`, `PlacardRenderer`): a plate on a wall, floor
//! or ceiling. Click it with an item to set what it holds (it is drawn on the plate); click it
//! again with an item that matches, or that a Filter item held in it accepts, and it gives a
//! 19 tick redstone pulse; left-click to take the item back. The rules are
//! `create_kinetics::rules::placard`; the hooks that give a signal are in the same style as
//! `analog_lever.rs`.
//!
//! Known differences from Create, for the README:
//! - A hook that gives a signal can read only the level's blocks, so a Placard's pulse is also
//!   kept in a table its block entity fills (as the Analog Lever's output is); a Placard in a chunk
//!   that has just loaded gives nothing until its block entity has ticked once.
//! - Setting the item takes one of the stack in the hand as `copyWithCount(1)` does, but Horizon
//!   can take only a plain stack (no components) from a hand inside a player's hook: a stack with
//!   components (a named, enchanted or damaged item) can be set only in Creative.
//! - `canSurvive` is checked every 20 ticks from the block entity (a placard whose support went
//!   falls off as an item), not on each neighbour change, and there is no waterlogging.
//! - The dust of a confirmed click is sent by the server (`send_particles`) instead of read by the
//!   client from the block entity's data.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::placard::{self as rules, Attach, Click};
use create_kinetics::rules::redstone_link::horizontal_angle;
use create_kinetics::{Dir, Pos, looking};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockSignalEvent, BlockStateId, Compound, DestroyBlockEvent,
    DisplayContext, Flow, Hooks, Host, IsSignalSourceEvent, ItemStack,
    LeftClickBlockEvent, LevelHost, ParticleOptions, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::diodes_signal as signal;
use crate::{int, util};

pub const ID: &str = "create:placard";

const ITEM: &str = "Item";
const POWERED_TICKS: &str = "PoweredTicks";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// How often a placard checks that what it is fixed to is still there.
const SUPPORT_EVERY: i64 = 20;
/// Ticks a placard may miss before it counts as gone.
const MAX_AGE: i64 = 3;

/// A server tick counter, for pruning.
static NOW: AtomicI64 = AtomicI64::new(0);

/// Whether each ticking placard is giving its pulse and when it last ticked, by level and position.
static SIGNALS: LazyLock<Mutex<HashMap<(u8, Pos), (bool, i64)>>> = LazyLock::new(Default::default);

struct Placard;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Placard)
}

/// Publishes whether a placard gives its pulse. True when the table had no entry for it.
fn publish(level: u8, pos: Pos, powered: bool) -> bool {
    let now = NOW.load(Ordering::Relaxed);
    match SIGNALS.lock() {
        Ok(mut table) => table.insert((level, pos), (powered, now)).is_none(),
        Err(_) => false,
    }
}

/// The plate's `face` and `facing` from its state.
fn orientation(host: &dyn Host, pos: Pos) -> Option<(Attach, Dir)> {
    let face = Attach::from_name(host.block_property(pos, "face")?.as_str())?;
    let facing = util::facing(host, pos, "facing")?;
    Some((face, facing))
}

/// The item a placard holds, from its saved data.
fn held(nbt: &Compound) -> Option<ItemStack> {
    nbt.get(&ITEM.into()).and_then(util::stack_of)
}

fn set_held(nbt: &mut Compound, stack: Option<&ItemStack>) {
    match stack {
        Some(stack) => {
            nbt.insert(ITEM.into(), util::stack_tag(stack));
        }
        None => {
            util::remove(nbt, ITEM);
        }
    }
}

/// `PlacardBlock.updateNeighbours`: the plate's neighbours and those of what it is fixed to.
fn tell_neighbours(host: &mut dyn Host, pos: Pos) {
    let dimension = host.dimension();
    host.update_signal_neighbors(dimension, pos);
}

fn play(host: &mut dyn Host, sound: &str, pos: Pos) {
    host.play_sound(sound, util::centre(pos), BLOCKS, 1.0, 1.0);
}

/// `PlacardBlockEntity.spawnParticles`.
fn particles(host: &mut dyn Host, pos: Pos) {
    let Some((face, facing)) = orientation(host, pos) else {
        return;
    };
    let connected = rules::connected_direction(face, facing);
    let centre = util::centre(pos);
    let options = ParticleOptions::dust([1.0, 0.2, 0.0], 1.0);
    for _ in 0..10 {
        let random = || (util::random() % 10_000) as f32 / 10_000.0 - 0.5;
        let offset = rules::particle_offset(connected, [random(), random(), random()]);
        let at = [
            centre[0] + f64::from(offset[0]),
            centre[1] + f64::from(offset[1]),
            centre[2] + f64::from(offset[2]),
        ];
        host.send_particles(&options, None, at, [0.0; 3], 0.0, 1);
    }
}

impl Hooks for Placard {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            let mut nbt = Compound::new();
            nbt.insert(POWERED_TICKS.into(), Tag::Int(0));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `PlacardBlockEntity.tick`.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let level = signal::host_level(host);
        let ticks = int(&event.entity.nbt, POWERED_TICKS);
        // A placard that has just loaded tells its neighbours what it gives.
        if publish(level, pos, ticks > 0) && ticks > 0 {
            tell_neighbours(host, pos);
        }
        if ticks > 0 {
            let (next, off) = rules::tick(ticks);
            event.entity.nbt.insert(POWERED_TICKS.into(), Tag::Int(next));
            if off {
                publish(level, pos, false);
                host.set_block_property(pos, "powered", "false");
                tell_neighbours(host, pos);
            }
        }
        // `canSurvive`: the block it is fixed to must have a collision shape.
        if util::game_time(host) % SUPPORT_EVERY == (i64::from(pos[0] ^ pos[2]) & 7) {
            self.check_support(host, pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        // An empty compound when nothing is held: the client merges what it is sent.
        let mut tag = Compound::new();
        let item = held(&event.entity.nbt).map_or_else(
            || Tag::Compound(Compound::new()),
            |stack| util::stack_tag(&stack),
        );
        tag.insert(ITEM.into(), item);
        Some(tag)
    }

    /// `PlacardRenderer`: the held item lies on the plate.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(stack) = event
            .entity
            .nbt
            .get(&ITEM.into())
            .and_then(util::stack_of)
        else {
            event.draw.keep();
            return;
        };
        let face = host
            .block_state_property(event.state, "face")
            .and_then(Attach::from_name)
            .unwrap_or(Attach::Wall);
        let facing = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
            .unwrap_or(Dir::North);
        // Stands in for `BakedModel.isGui3d`: an item that is also a block.
        let block_item = host.default_state(&stack.item).is_some();
        let pose = rules::item_pose(face, horizontal_angle(facing.normal()) as f32, block_item);
        let draw = &mut *event.draw;
        draw.push();
        draw.translate([0.5; 3]);
        draw.rotate_about([0.0, 1.0, 0.0], pose.y_degrees, [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], pose.x_degrees, [0.0; 3]);
        draw.translate([0.0, 0.0, pose.forward]);
        draw.scale([pose.scale; 3]);
        host.draw_item(draw, &stack, DisplayContext::Fixed, None, 0);
        draw.pop();
        draw.keep();
    }

    /// Prunes the placards that stopped ticking (their chunk unloaded, or the block went).
    fn server_tick(
        &self,
        _: &mut dyn Host,
        event: &mut horizon_client::mod_api::ServerTickEvent<'_>,
    ) {
        if let horizon_client::mod_api::ServerTickPhase::ServerTickStart { world_running: true } =
            event.phase
        {
            let now = NOW.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut table) = SIGNALS.lock() {
                table.retain(|_, (_, seen)| now - *seen < MAX_AGE);
            }
        }
    }

    /// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement`, turned round on a floor.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let (face, facing) = rules::placement(util::dir(context.face), looking(context.yaw, 0.0));
        let state = level.state_with_property(event.state, "face", face.name())?;
        level
            .state_with_property(state, "facing", facing.name())
            .map(Some)
    }

    /// `PlacardBlock.getSignal` and `getDirectSignal`.
    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        let key = (
            signal::level_index(event.dimension),
            [event.pos.x, event.pos.y, event.pos.z],
        );
        let powered = SIGNALS
            .lock()
            .ok()
            .and_then(|table| table.get(&key).map(|&(powered, _)| powered));
        if let Some(powered) = powered
            && let Some(face) = signal::choice(level, event.state, "face", &["floor", "wall", "ceiling"])
            && let Some(facing) = signal::horizontal_facing(level, event.state)
        {
            let face = Attach::from_name(face).unwrap_or(Attach::Wall);
            let connected = rules::connected_direction(face, facing);
            // `event.face` is the side that touches the reader: Java's side is its opposite.
            let towards_support = util::dir(event.face) == connected.opposite();
            event.signal = rules::signal(powered, event.direct, towards_support);
        }
        signal::mask(event);
    }

    /// `PlacardBlock.isSignalSource`: a block with a placard's three states; vanilla's levers and
    /// buttons have them too and are sources already.
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        if signal::choice(level, event.state, "face", &["floor", "wall", "ceiling"]).is_some()
            && signal::horizontal_facing(level, event.state).is_some()
            && signal::boolean(level, event.state, "powered").is_some()
        {
            event.source = true;
        }
    }

    /// `PlacardBlock.useItemOn`.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if &*event.stack.item == util::WRENCH {
            return util::wrench(host, pos, |_| {});
        }
        if player.sneaking || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Cancel;
        };
        let inside = held(&entity.nbt);
        let powered = int(&entity.nbt, POWERED_TICKS) > 0
            || host.block_property(pos, "powered").as_deref() == Some("true");
        let in_hand = event.stack;
        let outcome = rules::click(
            signal::may_build(player.game_mode),
            false,
            inside.is_none(),
            powered,
            || match &inside {
                // `FilterItem` held: `FilterItemStack.test`; else `isSameItemSameComponents`.
                Some(stack) if super::filter::is_filter_item(&stack.item) => {
                    super::filter::matches(&*host, stack, in_hand)
                }
                Some(stack) => util::same_item(stack, in_hand),
                None => false,
            },
        );
        match outcome {
            Click::Ignore => {}
            Click::Place => {
                let creative = player.game_mode == util::CREATIVE;
                let mut one = in_hand.clone();
                one.count = 1;
                // Only a plain stack can be taken from the hand here.
                if !creative && (in_hand.extra.is_some() || host.take_items(&in_hand.item, 1) != 1)
                {
                    return Flow::Cancel;
                }
                play(host, "minecraft:entity.item_frame.add_item", pos);
                host.change_mod_block_entity(pos, &mut |entity| {
                    set_held(&mut entity.nbt, Some(&one));
                });
                host.block_entity_sync(pos);
            }
            Click::Confirm => {
                play(host, "create:confirm", pos);
                let level = signal::host_level(host);
                host.change_mod_block_entity(pos, &mut |entity| {
                    entity.nbt.insert(POWERED_TICKS.into(), Tag::Int(rules::POWERED_TICKS));
                });
                publish(level, pos, true);
                host.set_block_property(pos, "powered", "true");
                tell_neighbours(host, pos);
                particles(host, pos);
            }
            Click::Deny => play(host, "create:deny", pos),
        }
        Flow::Cancel
    }

    /// `PlacardBlock.attack`: a left click puts what the placard holds into the player's inventory.
    fn left_click_block(&self, host: &mut dyn Host, event: &mut LeftClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let Some(inside) = host.mod_block_entity(pos).and_then(|entity| held(&entity.nbt)) else {
            return Flow::Continue;
        };
        host.change_mod_block_entity(pos, &mut |entity| set_held(&mut entity.nbt, None));
        host.block_entity_sync(pos);
        if let Some(rest) = host.give_stack(player.uuid, inside) {
            util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
        }
        play(host, "minecraft:entity.item_frame.remove_item", pos);
        Flow::Continue
    }

    /// `PlacardBlock.onRemove`: the held item is dropped with the block.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        if let Some(inside) = host.mod_block_entity(pos).and_then(|entity| held(&entity.nbt)) {
            host.change_mod_block_entity(pos, &mut |entity| set_held(&mut entity.nbt, None));
            util::spawn_item(host, util::centre(pos), &inside, [0.0; 3], 10);
        }
        Flow::Continue
    }
}

impl Placard {
    /// `PlacardBlock.canSurvive`: breaks the placard when the block it is fixed to has no
    /// collision shape.
    fn check_support(&self, host: &mut dyn Host, pos: Pos) {
        let Some((face, facing)) = orientation(host, pos) else {
            return;
        };
        let support = util::add(pos, rules::connected_direction(face, facing).opposite().normal());
        if !host.chunk_is_loaded(host.dimension(), [support[0] >> 4, support[2] >> 4]) {
            return;
        }
        if host.block_collision(support).is_empty() {
            host.destroy_block(pos, None, false);
        }
    }
}
