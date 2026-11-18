//! The Andesite and Brass Tunnels: a casing over a belt block that covers it, takes the shape of
//! what is beside the belt, hangs flaps that swing as items pass, and (Andesite) splits one item
//! off a stack to each side output or (Brass) distributes whole stacks by its mode
//! (`BeltTunnelBlock`, `BeltTunnelBlockEntity`, `BrassTunnelBlock`, `BrassTunnelBlockEntity`,
//! `BeltTunnelItem`, `BeltTunnelRenderer`, `BeltTunnelInteractionHandler`). The rules are
//! `create_kinetics::rules::tunnel` and `tunnel_brass`; `tunnel_world.rs` reads the world,
//! `tunnel_cross.rs` is what the belt calls as an item crosses under a tunnel and
//! `tunnel_distribute.rs` is a Brass Tunnel's tick.
//!
//! Known differences from Create:
//! - The shape, sides and flaps follow the neighbours on a timer (every 4 ticks and when the
//!   tunnel is placed), not on each neighbour update. A tunnel closed with the Wrench stays
//!   closed until its neighbours change what it could be (Create reopens it whenever a block
//!   beside the belt changes).
//! - The distribution mode and the filter of each open side are Create's value boxes (`machines/value_box.rs`);
//!   a filter is one item (no amount). The Wrench closes and opens a window as in Create; sneaking with it picks
//!   the tunnel up.
//! - An empty-hand click on a Brass Tunnel takes the stacks of its group, as Create's does. There
//!   is no goggle tooltip (the group's held stacks aren't sent to clients), no item handler (a hopper or funnel can't insert into a tunnel, and a
//!   Smart Observer or comparator doesn't see what it holds), no Display Link support, and no
//!   connected texture on the top of Brass Tunnels in a group.
//! - A tunnel is placed on a horizontal belt only and puts a casing on the belt where there is
//!   none (`BeltTunnelItem`); a tunnel whose belt loses its casing or is broken breaks a few ticks
//!   later instead of at once. Waterlogging is left out, as elsewhere.
//! - The flaps are drawn swinging out and settling as `FlapStuffs` moves them; no sound.
//! - Clients are told an item is waiting at a tunnel (`at_tunnel` on the belt's item) rather than
//!   working it out from the tunnel they can see, so a waiting stack is drawn standing.

use create_kinetics::rules::tunnel as rules;
use create_kinetics::rules::tunnel_brass::Mode;
use create_kinetics::{Axis, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, DestroyBlockEvent, EquipmentSlot, Flow, Hooks,
    Host, LevelHost, ModBlockEntity, PlaceBlockEvent, RightClickBlockEvent, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::tunnel_cross as cross;
use super::tunnel_distribute as distribute;
use super::tunnel_world::{self as world, ANDESITE, BRASS, View};
use crate::{belt, int, util};

/// `AllPartialModels.BELT_TUNNEL_FLAP`.
pub const FLAP: &str = "create:block/belt_tunnel/flap";
/// `BeltBlockEntity.CasingType` ordinals.
const ANDESITE_CASING: i32 = 1;
const BRASS_CASING: i32 = 2;
/// `SoundSource.PLAYERS`.
const PLAYERS: usize = 7;

struct Tunnels;

pub fn hooks() -> Box<dyn Hooks> {
    cross::register();
    Box::new(Tunnels)
}

fn salt(pos: Pos) -> i64 {
    (i64::from(pos[0]) * 31 + i64::from(pos[1]) * 17 + i64::from(pos[2]) * 7).rem_euclid(1000)
}

// --- keeping the shape and the sides ----------------------------------------------------------

/// Pops what a Brass Tunnel holds (`BrassTunnelBlockEntity.destroy`).
fn pop_stack(host: &mut dyn Host, pos: Pos, nbt: &Compound) {
    if let Some(stack) = world::read_state(pos, nbt).1 {
        util::spawn_item(host, util::centre(pos), &stack, [0.0; 3], 10);
    }
}

/// `BeltTunnelBlock.neighborChanged`, `getTunnelState` and `updateTunnelConnections` once:
/// breaks the tunnel when its belt is gone, follows the belt's axis and what is beside it, and
/// keeps the sides, the flaps and a Brass Tunnel's connections. True when the tunnel broke.
fn refresh(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, view: &View) -> bool {
    if !world::can_survive(&*host, pos) {
        if view.brass {
            pop_stack(host, pos, &entity.nbt);
        }
        host.destroy_block(pos, None, false);
        return true;
    }
    let mut view = *view;
    if let Some((axis, shape)) = world::wanted(&*host, pos, &view) {
        if axis != view.axis {
            host.set_block_property(pos, "axis", axis.name());
        }
        if shape != view.shape {
            host.set_block_property(pos, "shape", shape.name());
        }
        view.axis = axis;
        view.shape = shape;
    }
    let found = world::connections(&*host, pos, view.brass, view.axis, view.shape);
    let sides = world::dirs_tag(&found.sides);
    let flaps = world::dirs_tag(&found.flaps);
    let mut sync = false;
    if util::differs(&entity.nbt, world::SIDES, &sides) {
        entity.nbt.insert(world::SIDES.into(), sides);
    }
    if util::differs(&entity.nbt, world::FLAPS, &flaps) {
        entity.nbt.insert(world::FLAPS.into(), flaps);
        sync = true;
    }
    if view.brass {
        for (key, now, left) in [
            (world::CONNECTED_LEFT, found.left, true),
            (world::CONNECTED_RIGHT, found.right, false),
        ] {
            if util::boolean(&entity.nbt, key) == now {
                continue;
            }
            entity.nbt.insert(key.into(), Tag::Byte(i8::from(now)));
            // `updateTunnelConnections`: a tunnel that joined the group takes our mode.
            if now && let Some(other) = world::adjacent(&*host, pos, view.axis, left) {
                let mode = world::mode_of(&entity.nbt);
                host.change_mod_block_entity(other, &mut |e| {
                    if int(&e.nbt, world::MODE) != mode.ordinal() {
                        e.nbt.insert(world::MODE.into(), Tag::Int(mode.ordinal()));
                    }
                });
            }
        }
    }
    if sync {
        host.block_entity_sync(pos);
    }
    false
}

// --- drawing ------------------------------------------------------------------------------------

/// `BeltTunnelRenderer`: each flap swings as the last item moved it.
fn draw(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let nbt = &event.entity.nbt;
    let flaps = world::dirs_of(nbt, world::FLAPS);
    let game_time = event.draw.game_time;
    let partial = event.draw.partial;
    let mut swinging = false;
    let draw = &mut *event.draw;
    for side in flaps {
        let flapness = match world::swing_of(nbt, side) {
            Some((inward, tick)) => {
                let since = (game_time - tick) as f32 + partial;
                if since < rules::FLAP_TICKS {
                    swinging = true;
                    rules::flap_value(inward, since)
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        // `FlapStuffs.renderFlaps`.
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], rules::horizontal_angle(side.opposite()), [0.5; 3]);
        draw.translate([rules::FLAP_X_OFFSET, 0.0, 0.0]);
        for segment in 0..rules::FLAP_COUNT {
            draw.push();
            draw.rotate_about([1.0, 0.0, 0.0], rules::flap_angle(flapness, segment), rules::FLAP_PIVOT);
            host.draw_model(draw, FLAP, 0xFF_FFFF);
            draw.pop();
            draw.translate([rules::FLAP_SEGMENT_STEP, 0.0, 0.0]);
        }
        draw.pop();
    }
    if !swinging {
        draw.keep();
    }
}

// --- the hooks ----------------------------------------------------------------------------------

impl Hooks for Tunnels {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        if !event.models.iter().any(|m| m == FLAP) {
            event.models.push(FLAP.to_owned());
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        world::is_tunnel(event.block).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if world::is_tunnel(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !world::is_tunnel(&event.entity.id) {
            return;
        }
        let pos = event.pos;
        let Some(view) = world::view(&*host, pos) else {
            return;
        };
        let now = util::game_time(host);
        if !util::has(&event.entity.nbt, world::SIDES) || (now + salt(pos)) % world::STATE_RATE == 0 {
            if refresh(host, pos, event.entity, &view) {
                return;
            }
        }
        if view.brass {
            distribute::tick(host, pos, event.entity);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if !world::is_tunnel(&event.entity.id) {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        tag.insert(world::FLAPS.into(), world::dirs_tag(&world::dirs_of(nbt, world::FLAPS)));
        // Always sent, even empty, so a client that had swings drops them.
        let swings = match util::get(nbt, world::SWINGS) {
            Some(swings) => swings.clone(),
            None => Tag::List(Vec::new()),
        };
        tag.insert(world::SWINGS.into(), swings);
        Some(tag)
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if world::is_tunnel(event.block) {
            draw(host, event);
        }
    }

    /// `BeltTunnelItem.canPlace`: only over a horizontal belt. The axis is the belt's
    /// (`getTunnelState`); the shape follows once it is placed.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if !matches!(&*context.item, ANDESITE | BRASS) {
            return None;
        }
        let under = level.block_state([context.pos.x, context.pos.y - 1, context.pos.z]);
        if level.state_with_property(under, "slope", "horizontal") != Some(under) {
            return Some(None);
        }
        let along = |facing: &str| level.state_with_property(under, "facing", facing) == Some(under);
        let axis = if along("east") || along("west") { Axis::X } else { Axis::Z };
        level.state_with_property(event.state, "axis", axis.name()).map(Some)
    }

    /// `BeltTunnelItem.updateCustomBlockEntityTag`: a belt with no casing gets this tunnel's.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        let brass = match &*event.stack.item {
            ANDESITE => false,
            BRASS => true,
            _ => return,
        };
        let under = world::below(event.pos);
        if belt::below(&*host, under).is_some() && belt::casing(&*host, under) == 0 {
            belt::set_casing(host, under, if brass { BRASS_CASING } else { ANDESITE_CASING });
        }
    }

    /// A Brass Tunnel's stack comes out with it (`BrassTunnelBlockEntity.destroy`).
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != BRASS {
            return Flow::Continue;
        }
        if let Some(entity) = host.mod_block_entity(pos) {
            let nbt = entity.nbt.clone();
            pop_stack(host, pos, &nbt);
            host.change_mod_block_entity(pos, &mut |e| {
                util::remove(&mut e.nbt, world::STACK);
            });
        }
        Flow::Continue
    }

    /// The Wrench (`BeltTunnelBlock.onWrenched`, `IWrenchable.onSneakWrenched`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let Some(view) = world::view(&*host, pos) else {
            return Flow::Continue;
        };
        if &*event.stack.item == util::WRENCH {
            return util::wrench(host, pos, |host| {
                if let Some(next) = rules::wrenched(view.shape) {
                    host.set_block_property(pos, "shape", next.name());
                }
            });
        }
        Flow::Continue
    }

    /// With an empty hand the stacks the group is distributing come back to the player
    /// (`BrassTunnelBlock.useWithoutItem`).
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != BRASS {
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
        if distribute::grab_group(host, pos, true).is_empty() {
            return Flow::Continue;
        }
        if !util::first_click(&*host, uuid, pos) {
            return Flow::Cancel;
        }
        for stack in distribute::grab_group(host, pos, false) {
            if let Some(rest) = host.give_stack(uuid, stack) {
                util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
            }
        }
        let pitch = 1.0 + (util::random() % 1000) as f32 / 1000.0;
        host.play_sound("minecraft:entity.item.pickup", util::centre(pos), PLAYERS, 0.2, pitch);
        Flow::Cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blocks_are_told_apart() {
        assert!(world::is_tunnel(ANDESITE) && world::is_tunnel(BRASS));
        assert!(!world::is_tunnel("create:belt"));
    }

    #[test]
    fn a_fresh_tunnel_splits() {
        assert_eq!(world::mode_of(&Compound::new()), Mode::Split);
    }
}
