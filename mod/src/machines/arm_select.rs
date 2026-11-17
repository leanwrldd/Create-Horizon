//! Choosing a Mechanical Arm's points with the item (`ArmInteractionPointHandler`, `ArmItem`,
//! `ArmPlacementPacket`): right-click a block with the arm in the main hand to select it as an
//! input, again for an output; left-click to forget it; place the arm and it takes what is in
//! range. The rules are `create_kinetics::rules::arm_points::Selection`; this keeps one per
//! player, tells them, and writes the points onto the arm placed. See `arm.rs` for the list of
//! known differences.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::arm_points::{self as points, Selection};
use crate::compat::{
    EquipmentSlot, Flow, Hooks, Host, LeftClickBlockEvent, ParticleOptions, PlaceBlockEvent,
    RightClickBlockEvent, ServerTickEvent, ServerTickPhase, Text,
};

use super::arm::{ID, POINTS, points_tag};
use super::arm_world::classify_at;
use crate::util;

const SPECTATOR: i32 = 3;
/// Ticks between the dust that marks the selected blocks.
const MARK_RATE: i64 = 10;
/// `ChatFormatting.RED` and `WHITE`.
const RED: u32 = 0xFF5555;
const WHITE: u32 = 0xFFFFFF;

struct Select;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Select)
}

/// Server ticks seen, for the dust's rate (a server tick has no level to read the game time from).
static TICKS: AtomicI64 = AtomicI64::new(0);

/// Each player's `currentSelection`.
static SELECTIONS: LazyLock<Mutex<HashMap<u128, Selection>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn selections() -> std::sync::MutexGuard<'static, HashMap<u128, Selection>> {
    SELECTIONS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn holds_arm(host: &dyn Host, uuid: u128) -> bool {
    host.player_equipment(uuid, EquipmentSlot::MainHand).is_some_and(|s| &*s.item == ID)
}

fn say(host: &mut dyn Host, uuid: u128, line: String, color: u32) {
    host.action_bar(uuid, Text::colored(line, color));
}

impl Hooks for Select {
    /// `rightClickingBlocksSelectsThem`: a block that makes a point is selected (or its mode
    /// cycled) and the click ends there; any other block is used as usual.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == SPECTATOR || !holds_arm(&*host, player.uuid) {
            return Flow::Continue;
        }
        let pos = event.pos;
        // The off hand's turn of a click that already selected it (or cycled its mode).
        if util::click_repeated(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let ty = classify_at(&*host, pos);
        let chosen = selections().entry(player.uuid).or_default().right_click(pos, ty);
        let Some((_, mode)) = chosen else {
            return Flow::Continue;
        };
        util::click_taken(&*host, player.uuid, pos);
        let name = points::block_title(&host.block(pos));
        say(host, player.uuid, mode.line(&name), mode.color());
        Flow::Cancel
    }

    /// `leftClickingBlocksDeselectsThem`, and `ArmItem.canAttackBlock`: the arm doesn't break the
    /// blocks it can point at.
    fn left_click_block(&self, host: &mut dyn Host, event: &mut LeftClickBlockEvent) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !holds_arm(&*host, player.uuid) {
            return Flow::Continue;
        }
        let forgot = selections().get_mut(&player.uuid).is_some_and(|s| s.left_click(event.pos));
        if forgot || classify_at(&*host, event.pos).is_some() {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    /// `ArmItem.updateCustomBlockEntityTag` and `ArmInteractionPointHandler.flushSettings`: the
    /// arm placed takes the points in range, and the player is told what it got.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID || host.block(event.pos) != ID {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let selection = selections().remove(&player.uuid).unwrap_or_default();
        let flush = selection.flush(event.pos);
        let tag = points_tag(&flush.points, event.pos);
        let pos = event.pos;
        host.change_mod_block_entity(pos, &mut |entity| {
            entity.nbt.insert(POINTS.into(), tag.clone());
        });
        host.block_entity_sync(pos);
        if let Some((line, red)) = flush.message() {
            say(host, player.uuid, line, if red { RED } else { WHITE });
        }
    }

    /// The selection ends when the arm leaves the hand (`currentItem = null`); until then the chosen
    /// blocks are marked with dust (Create outlines them) in the colour of their mode.
    fn server_tick(&self, host: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        let ServerTickPhase::ServerTickStart { world_running: true } = event.phase else {
            return;
        };
        let now = TICKS.fetch_add(1, Ordering::Relaxed);
        if now.rem_euclid(MARK_RATE) != 0 {
            return;
        }
        let players: Vec<u128> = selections().keys().copied().collect();
        for uuid in players {
            if !holds_arm(&*host, uuid) {
                selections().remove(&uuid);
                continue;
            }
            let marked = selections().get(&uuid).map(|s| s.points.clone()).unwrap_or_default();
            for point in marked {
                let [r, g, b] = [16, 8, 0].map(|shift| ((point.mode.color() >> shift) & 0xFF) as f32 / 255.0);
                let options = ParticleOptions::dust([r, g, b], 0.6);
                for corner in 0..8 {
                    let at = [0, 1, 2].map(|axis| {
                        f64::from(point.pos[axis]) + if corner >> axis & 1 == 0 { 0.02 } else { 0.98 }
                    });
                    host.send_particles_to(uuid, &options, None, at, [0.0; 3], 0.0, 1, false);
                }
            }
        }
    }
}
