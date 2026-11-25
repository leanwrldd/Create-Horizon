//! Create's value boxes, written once for every block (`ValueBoxTransform`, `ValueSettingsBehaviour`,
//! `ValueSettingsInputHandler`, `ValueSettingsPacket`, `ScrollValueBehaviour`, `ScrollOptionBehaviour`,
//! `FilteringBehaviour`). This file is the shared part and the server's: it finds the box a click lands on,
//! applies a short click to a filter slot, and applies the setting a client's value board ends on. The client's
//! drawing is `value_box_draw.rs`, its click, hold and board `value_box_screen.rs`. The boxes of each block, their
//! positions, ranges and option lists are the rules in `create_kinetics::rules::value_box*`.
//!
//! How a player uses a box (Create 6.0.10's way):
//! - Look at a block with boxes: its boxes show (a number, an option's name, a filter's amount) and the one the
//!   crosshair is on is outlined, with its label and a hint at the bottom of the screen.
//! - A **filter slot** without an amount: right-click it with an item to set that item as the filter (a List or
//!   Attribute Filter is taken from your hand and the old one given back), with an empty hand to clear it.
//! - A **filter slot with an amount** (a Smart Chute that pulls, a Brass Funnel that extracts), a **scroll value**
//!   (the Creative Motor's speed, the Rotation Speed Controller's target) and a **scroll option** (a Brass Tunnel's
//!   distribution, the Mechanical Arm's selection mode): click and hold the right button on the box. Let go within
//!   a quarter of a second and it is a short click (a filter is set or cleared as above; a number or option does
//!   nothing); hold on and the value board opens: slide along the bar (or use the mouse wheel, Shift for the marked
//!   steps) and let go of the button to set it. Escape leaves it as it was.
//! - Sneaking, or being in adventure or spectator mode, leaves the box alone (`ValueSettingsInputHandler.canInteract`).
//!
//! Known differences from Create, for the README:
//! - Create's client cancels the right click while the hold is in progress. Here the server takes the click on a
//!   box with a board at once (nothing happens on press) and the client decides at the release whether it was a
//!   short click (`create:value_box` message) or a board; a filter slot with no amount acts on the press, as Create's
//!   does. The server's own ray (the player's eye against the block's collision boxes) stands in for
//!   `BlockHitResult.getLocation`, which `RightClickBlockEvent` doesn't carry.
//! - A box on a block whose collision shape differs from its outline can be missed by a few pixels at its edge.
//! - Clipboard copy and paste of settings (`ValueSettingsBehaviour.writeToClipboard`) and the bulk edit with Ctrl
//!   (`BulkScrollValueBehaviour`) are not ported.
//! - Create's `Dual` box transform is not ported (nothing here uses it).

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use create_kinetics::rules::chute as chute_rules;
use create_kinetics::rules::value_box::{self as geo, Props, Slot, Vec3};
use create_kinetics::rules::value_box_blocks::{self as table, BoxDef, Kind};
use create_kinetics::rules::value_box_board::{self as board, Values};
use create_kinetics::rules::value_box_input::{self as input, Hand, Message, Short};
use create_kinetics::{Axis, Dir, Pos};
use crate::compat::{
    Compound, Flow, Hooks, Host, ItemStack, LevelTickPostEvent, NativeInventoryKind, NativeServerPlayer, NetFromClientEvent,
    PlayerTickEvent, RightClickBlockEvent, Tag,
};

use super::{chute_inv, filter, smart_fluid_pipe, tunnel_distribute, tunnel_world};
use crate::{int, util};

/// `Player.blockInteractionRange` plus a little for lag: how far a box may be from the player's eyes.
pub const REACH: f64 = 6.5;
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

pub const SCROLL_VALUE: &str = "ScrollValue";
pub const FILTER: &str = "Filter";
pub const FILTER_AMOUNT: &str = "FilterAmount";
pub const UP_TO: &str = "UpTo";
pub const FILTERS: &str = "Filters";

struct ValueBox;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(ValueBox)
}

// --- reading the block ------------------------------------------------------------------------------

/// What a slot reads of the block at `pos`: its state's properties, and the facts that `fact:` keys stand for
/// (the rules say which a box needs). `nbt` is the block entity's data, from the server's saved copy or the
/// client's update tag.
pub fn with_props_nbt<R>(host: &dyn Host, pos: Pos, block: &str, nbt: Option<&Compound>, f: impl FnOnce(&Props) -> R) -> R {
    let get = |key: &str| -> Option<String> {
        match key.strip_prefix("fact:") {
            Some(fact) => fact_of(host, pos, block, nbt, fact),
            None => host.block_property(pos, key),
        }
    };
    f(&Props::new(block, &get))
}

/// [`with_props_nbt`] for a block entity that draws itself with the state it is given, which is not always the one at
/// `pos` in the world (a Ponder scene's blocks are not in it).
pub fn with_state_nbt<R>(
    host: &dyn Host,
    state: crate::compat::BlockStateId,
    pos: Pos,
    block: &str,
    nbt: Option<&Compound>,
    f: impl FnOnce(&Props) -> R,
) -> R {
    let get = |key: &str| -> Option<String> {
        match key.strip_prefix("fact:") {
            Some(fact) => fact_of(host, pos, block, nbt, fact),
            None => host.block_state_property(state, key).map(str::to_owned),
        }
    };
    f(&Props::new(block, &get))
}

/// [`with_props_nbt`] with the block entity read from the host, its data handed on too.
pub fn with_props<R>(host: &dyn Host, pos: Pos, block: &str, f: impl FnOnce(&Props, Option<&Compound>) -> R) -> R {
    let entity = host.mod_block_entity(pos);
    let nbt = entity.as_ref().map(|e| &e.nbt);
    with_props_nbt(host, pos, block, nbt, |p| f(p, nbt))
}

fn flag(on: bool) -> Option<String> {
    Some(on.to_string())
}

/// The facts the box tables ask for (`fact:<name>`): what Create reads from the world and the block entity.
fn fact_of(host: &dyn Host, pos: Pos, block: &str, nbt: Option<&Compound>, fact: &str) -> Option<String> {
    match fact {
        // `SmartChuteBlockEntity.isExtracting`: the block the chute pulls from (above while items fall, below
        // while they rise) is not a chute and can't be built over.
        "extracting" => {
            let nbt = nbt?;
            let push = crate::float(nbt, "Push").unwrap_or(0.0);
            let pull = crate::float(nbt, "Pull").unwrap_or(0.0);
            let up = chute_rules::item_motion(push, pull) < 0.0;
            let source = host.block(util::add(pos, if up { Dir::Up } else { Dir::Down }.normal()));
            flag(!matches!(source.as_str(), "create:chute" | "create:smart_chute") && !table::is_replaceable(&source))
        }
        // `FunnelBlockEntity.supportsAmountOnFilter`: `BeltHelper.getSegmentBE(level, pos.below())`.
        "belt_below" => flag(host.block(util::add(pos, Dir::Down.normal())) == "create:belt"),
        // `BrassTunnelBlockEntity.hasDistributionBehaviour`.
        "distribution" => {
            let axis = host.block_property(pos, "axis").as_deref().and_then(Axis::from_name).unwrap_or(Axis::X);
            flag(tunnel_world::distributes(nbt?, axis))
        }
        // `BrassTunnelBlockEntity.isValidFaceForFilter`: the sides the tunnel opens on.
        side if side.starts_with("side_") => {
            let dir = Dir::from_name(&side["side_".len()..])?;
            flag(tunnel_world::dirs_of(nbt?, tunnel_world::SIDES).contains(&dir))
        }
        _ => {
            let _ = block;
            None
        }
    }
}

// --- where a click landed --------------------------------------------------------------------------------

/// The face of the block a ray met, and where, in the block's own coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub side: Dir,
    pub local: Vec3,
}

/// What `BlockHitResult` says, found from the eye ray against the block's collision boxes; where the block
/// has none, the point the ray crosses the face `face` the game reported.
pub fn hit_of(host: &dyn Host, eye: Vec3, look: Vec3, pos: Pos, face: Pos) -> Option<Hit> {
    let origin = [eye[0] - f64::from(pos[0]), eye[1] - f64::from(pos[1]), eye[2] - f64::from(pos[2])];
    let boxes = host.block_collision(pos);
    if let Some((local, side)) = geo::ray_boxes(&boxes, origin, look, REACH + 2.0) {
        return Some(Hit { side, local });
    }
    let side = Dir::from_normal(face)?;
    let at = create_kinetics::rules::display_link::hit_on_face(eye, look, pos, face);
    Some(Hit { side, local: [at[0] - f64::from(pos[0]), at[1] - f64::from(pos[1]), at[2] - f64::from(pos[2])] })
}

fn eye_of(player: &NativeServerPlayer) -> (Vec3, Vec3) {
    let eye = [player.position[0], player.position[1] + f64::from(player.eye_height), player.position[2]];
    (eye, geo::view_vector(f64::from(player.yaw), f64::from(player.pitch)))
}

/// `ValueSettingsInputHandler.canInteract`: not sneaking, not a spectator, not in adventure mode.
pub fn can_interact(player: &NativeServerPlayer) -> bool {
    !player.sneaking && !matches!(player.game_mode, util::SPECTATOR | 2)
}

// --- the values a box holds -------------------------------------------------------------------------------

/// The filter item of box `def` on `side`.
pub fn filter_item(nbt: &Compound, def: &BoxDef, side: Dir) -> Option<ItemStack> {
    if def.slot == Slot::BrassTunnelFilter {
        tunnel_world::filter_item(nbt, side)
    } else {
        chute_inv::filter_of(nbt).item
    }
}

/// The numbers saved for the box (`ScrollValue`, `FilterAmount`, `UpTo`), with Create's defaults and
/// `FilteringBehaviour.read`'s migration of a 0 amount.
pub fn values_of(host: &dyn Host, nbt: &Compound, def: &BoxDef, side: Dir) -> Values {
    let scroll = if util::has(nbt, SCROLL_VALUE) { int(nbt, SCROLL_VALUE) } else { table::default_value(def) };
    let max_stack = filter_item(nbt, def, side)
        .and_then(|stack| host.max_stack_size(&stack.item))
        .map_or(64, |max| max as i32);
    let amount = chute_inv::filter_of(nbt).amount;
    let (count, up_to) = if amount.count == 0 { (max_stack, true) } else { (amount.count, amount.up_to) };
    Values { scroll, count, up_to, max_stack }
}

/// Whether a box takes a value board: always for a scroll value or option; for a filter, when the block
/// shows its amount and the item stacks (`FilteringBehaviour.acceptsValueSettings` = `isCountVisible`).
pub fn takes_board(def: &BoxDef, p: &Props, values: &Values) -> bool {
    match def.kind {
        Kind::Value { .. } => true,
        Kind::Filter { .. } => def.shows_count(p) && values.max_stack > 1,
    }
}

// --- storing ------------------------------------------------------------------------------------------------

fn play_feedback(host: &mut dyn Host, pos: Pos) {
    // `ValueSettingsBehaviour.playFeedbackSound`.
    host.play_sound("minecraft:entity.item_frame.add_item", util::centre(pos), BLOCKS, 0.25, 2.0);
    host.play_sound("minecraft:block.note_block.iron_xylophone", util::centre(pos), BLOCKS, 0.03, 1.125);
}

/// `ScrollValueBehaviour.setValue`: saves the number and tells what has to follow it.
fn write_scroll(host: &mut dyn Host, pos: Pos, block: &str, value: i32) {
    host.change_mod_block_entity(pos, &mut |entity| {
        if int(&entity.nbt, SCROLL_VALUE) != value || !util::has(&entity.nbt, SCROLL_VALUE) {
            entity.nbt.insert(SCROLL_VALUE.into(), Tag::Int(value));
        }
    });
    // `ScrollOptionBehaviour` callbacks: a Brass Tunnel passes its mode to the tunnels beside it.
    if block == "create:brass_tunnel" {
        tunnel_distribute::set_mode_group(host, pos, create_kinetics::rules::tunnel_brass::Mode::from_ordinal(value));
    }
    // `RollerBlockEntity.onModeChanged`: the rollers along the same line take the mode and filter too.
    if block == "create:mechanical_roller" {
        super::roller::share_to_adjacent(host, pos);
    }
    host.block_entity_sync(pos);
}

/// `FilteringBehaviour.setFilter`: the item (one of it) becomes the filter, a new item keeping the amount.
fn write_filter(host: &mut dyn Host, pos: Pos, def: &BoxDef, side: Dir, item: Option<&ItemStack>) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let one = item.map(|stack| {
        let mut one = stack.clone();
        one.count = 1;
        one
    });
    if def.slot == Slot::BrassTunnelFilter {
        host.change_mod_block_entity(pos, &mut |e| tunnel_world::set_filter(&mut e.nbt, side, one.as_ref()));
    } else if def.slot == Slot::SmartPipe {
        // The pipe also saves the fluid the item stands for (`FilterFluid`), which the pumps read.
        smart_fluid_pipe::set_filter(host, pos, one);
    } else {
        let mut current = chute_inv::filter_of(&entity.nbt);
        let new_max = one.as_ref().and_then(|stack| host.max_stack_size(&stack.item)).map(|m| m as i32);
        let after = input::after_set(&values_of(&*host, &entity.nbt, def, side), new_max);
        current.item = one;
        current.amount = chute_rules::Amount { count: after.count, up_to: after.up_to };
        host.change_mod_block_entity(pos, &mut |e| chute_inv::write_filter(&mut e.nbt, &current));
        // `RollerBlockEntity.onFilterChanged`.
        if matches!(def.slot, Slot::Roller { .. }) {
            super::roller::share_to_adjacent(host, pos);
        }
    }
    host.block_entity_sync(pos);
}

/// What the player's main hand holds, as a short click sees it.
fn hand_of(host: &dyn Host, uuid: u128) -> (Hand, Option<ItemStack>) {
    let stack = host.held_stack(uuid);
    let hand = match &stack {
        None => Hand { empty: true, ..Hand::default() },
        Some(stack) => Hand {
            empty: false,
            wrench: &*stack.item == util::WRENCH,
            arm: &*stack.item == "create:mechanical_arm",
            filter_item: filter::is_filter_item(&stack.item),
        },
    };
    (hand, stack)
}

/// The Filter items a survival player has set and owes a shrink for, and when: applied in `level_tick_post`
/// (a player's own hook can't change an inventory slot).
static SHRINK: Mutex<Vec<(u128, usize, String)>> = Mutex::new(Vec::new());

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `FilteringBehaviour.onShortInteract`: the held item becomes the filter, an empty hand clears it.
pub fn short_click(host: &mut dyn Host, player: &NativeServerPlayer, pos: Pos, def: &BoxDef, side: Dir) {
    let uuid = player.uuid;
    let (hand, stack) = hand_of(&*host, uuid);
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let old = filter_item(&entity.nbt, def, side);
    let old_is_filter = old.as_ref().is_some_and(|stack| filter::is_filter_item(&stack.item));
    let creative = player.game_mode == util::CREATIVE;
    // `ItemHelper.extract(inventory, same item, simulate)`: only the plain stacks of the inventory can be told.
    let creative_copy = creative
        && old
            .as_ref()
            .is_some_and(|old| host.carried_items().iter().any(|(item, count)| item.as_str() == &*old.item && *count > 0));
    let Short::Set { refund, consume } = input::short_interact(hand, old_is_filter, creative, creative_copy) else {
        return;
    };
    if refund && let Some(old) = &old {
        let mut back = old.clone();
        back.count = 1;
        if let Some(rest) = host.give_stack(uuid, back) {
            util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
        }
    }
    let new = stack.as_ref().filter(|_| !hand.empty).map(|held| {
        let mut one = held.clone();
        one.count = 1;
        one
    });
    // `FilteringBehaviour.setFilter`'s predicate: the Roller takes only a block it can pave with.
    if matches!(def.slot, Slot::Roller { .. })
        && let Some(item) = &new
        && !super::roller::accepts(&*host, &item.item)
    {
        super::roller::deny(host, player);
        return;
    }
    write_filter(host, pos, def, side, new.as_ref());
    if consume && let Some(held) = &stack {
        lock(&SHRINK).push((uuid, usize::from(player.selected_slot), held.item.to_string()));
    }
    host.play_sound("minecraft:entity.item_frame.add_item", util::centre(pos), BLOCKS, 0.25, 0.1);
}

/// `ValueSettingsBehaviour.setValueSettings`, from a settings message already checked.
fn apply_settings(host: &mut dyn Host, pos: Pos, block: &str, def: &BoxDef, side: Dir, settings: board::Settings) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let values = values_of(&*host, &entity.nbt, def, side);
    let Some(behaviour) = def.behaviour() else {
        return;
    };
    let Some(next) = board::apply(behaviour, &values, settings) else {
        return;
    };
    match def.kind {
        Kind::Value { .. } => write_scroll(host, pos, block, next.scroll),
        Kind::Filter { .. } => {
            let mut current = chute_inv::filter_of(&entity.nbt);
            current.amount = chute_rules::Amount { count: next.count, up_to: next.up_to };
            host.change_mod_block_entity(pos, &mut |e| chute_inv::write_filter(&mut e.nbt, &current));
            host.block_entity_sync(pos);
        }
    }
    play_feedback(host, pos);
}

// --- the click ----------------------------------------------------------------------------------------------

/// `ValueSettingsInputHandler.onBlockActivated`, called first for every right click on a block (from
/// `Core`'s `right_click_block`, so a box takes priority over what the block does on a click). A click on a box
/// is taken whatever the item in hand; a filter slot with no amount acts at once, any other waits for the
/// client's release or board.
pub fn right_click(host: &mut dyn Host, event: &RightClickBlockEvent) -> Flow {
    let pos = event.pos;
    let block = host.block(pos);
    if !table::has_boxes(&block) {
        return Flow::Continue;
    }
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    if !can_interact(&player) {
        return Flow::Continue;
    }
    let (eye, look) = eye_of(&player);
    let Some(hit) = hit_of(&*host, eye, look, pos, event.face) else {
        return Flow::Continue;
    };
    let found = with_props(&*host, pos, &block, |p, nbt| {
        let def = table::hit_box(&block, p, hit.side, hit.local)?;
        let board = nbt.is_some_and(|nbt| takes_board(def, p, &values_of(&*host, nbt, def, hit.side)));
        Some((def, board))
    });
    let Some((def, board)) = found else {
        return Flow::Continue;
    };
    if util::click_repeated(&*host, player.uuid, pos) {
        return Flow::Cancel;
    }
    util::click_taken(&*host, player.uuid, pos);
    if def.is_filter() && !board {
        short_click(host, &player, pos, def, hit.side);
    }
    Flow::Cancel
}

// --- what a client says --------------------------------------------------------------------------------------

/// Messages waiting for their player's tick, by player (`net_from_client` has no player's state swapped in).
static INBOX: Mutex<BTreeMap<u128, Vec<Message>>> = Mutex::new(BTreeMap::new());

fn handle(host: &mut dyn Host, player: &NativeServerPlayer, message: Message) {
    let (pos, net_id, side, settings) = match message {
        Message::Short { pos, net_id, side } => (pos, net_id, side, None),
        Message::Set { pos, net_id, side, row, value } => (pos, net_id, side, Some(board::Settings { row, value })),
    };
    let Some(side) = Dir::ALL.get(usize::from(side)).copied() else {
        return;
    };
    if !can_interact(player) {
        return;
    }
    let eye = [player.position[0], player.position[1] + f64::from(player.eye_height), player.position[2]];
    let centre = util::centre(pos);
    let near = (0..3).map(|i| (eye[i] - centre[i]).powi(2)).sum::<f64>().sqrt();
    if near > REACH + 1.5 {
        return;
    }
    let block = host.block(pos);
    let found = with_props(&*host, pos, &block, |p, nbt| {
        let def = table::boxes(&block).iter().find(|d| d.net_id == net_id && (d.active)(p) && d.slot.exists(p, side))?;
        Some((def, takes_board(def, p, &values_of(&*host, nbt?, def, side))))
    });
    let Some((def, board)) = found else {
        return;
    };
    match settings {
        None if def.is_filter() => short_click(host, player, pos, def, side),
        None => {}
        Some(settings) if board => apply_settings(host, pos, &block, def, side, settings),
        Some(_) => {}
    }
}

impl Hooks for ValueBox {
    fn net_from_client(&self, _: &mut dyn Host, event: &mut NetFromClientEvent<'_>) {
        if event.channel != input::CHANNEL {
            return;
        }
        if let Some(message) = Message::decode(event.data) {
            let mut inbox = lock(&INBOX);
            let queue = inbox.entry(event.player).or_default();
            if queue.len() < 16 {
                queue.push(message);
            }
        }
    }

    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let messages = lock(&INBOX).remove(&player.uuid).unwrap_or_default();
        for message in messages {
            handle(host, &player, message);
        }
    }

    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        let owed: Vec<(u128, usize, String)> = std::mem::take(&mut *lock(&SHRINK));
        for (uuid, slot, item) in owed {
            let Some(stack) = host.held_stack(uuid).filter(|s| *s.item == *item) else {
                continue;
            };
            let new = (stack.count > 1).then(|| {
                let mut less = stack.clone();
                less.count -= 1;
                less
            });
            let done = host
                .set_inventory_slot_typed(uuid, NativeInventoryKind::Player, slot, Some(&stack), new)
                .is_ok();
            if !done {
                lock(&SHRINK).push((uuid, slot, item));
            }
        }
    }
}

// --- the update tag ---------------------------------------------------------------------------------------------

/// Keys a client needs to draw a block's boxes, copied into every module's update tag (`forward.rs` calls
/// this, so a module needn't send them): the saved filter, its amount, the scroll value, a tunnel's sided
/// filters and what a tunnel's boxes depend on. Each is sent only when the entity has it, and an
/// empty filter is sent as an empty compound so a client that had one drops it.
pub fn decorate(block: &str, nbt: &Compound, tag: &mut Compound) {
    if !table::has_boxes(block) {
        return;
    }
    const KEYS: &[&str] = &[SCROLL_VALUE, FILTER, FILTER_AMOUNT, UP_TO, FILTERS];
    for key in KEYS {
        if !util::has(tag, key)
            && let Some(value) = util::get(nbt, key)
        {
            tag.insert((*key).into(), value.clone());
        }
    }
    if block == "create:brass_tunnel" {
        for key in [
            tunnel_world::SIDES,
            tunnel_world::FLAPS,
            tunnel_world::CONNECTED_LEFT,
            tunnel_world::CONNECTED_RIGHT,
        ] {
            if !util::has(tag, key)
                && let Some(value) = util::get(nbt, key)
            {
                tag.insert(key.into(), value.clone());
            }
        }
    }
}
