//! Aiming a Display Link with the item (`ClickToLinkBlockItem`, `DisplayLinkBlockItem`): with the
//! link in hand, right-click the block to write to (its first click only selects it), then
//! right-click where the link should go; the link is placed with that block as its target if it
//! is within 64 blocks. Sneak-right-click forgets the selection. Like the Mechanical Arm's
//! (`arm_select.rs`), the selection is kept per player, not on the item, and shown by dust at
//! the block's corners (Create outlines it).
//!
//! Known differences from Create, for the README:
//! - The selection belongs to the player, not the item stack, and ends when the link leaves the
//!   main hand. It isn't saved. A multi-block target (a board, a row of nixie tubes) is marked at
//!   the block clicked, not over the whole of it.
//! - The distance is measured from where the link is placed to the block selected, as Create's
//!   `closerThan` does; a click too far away is refused with a red message and places nothing.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use create_kinetics::Pos;
use create_kinetics::rules::display_link::within_range;
use crate::compat::{
    EquipmentSlot, Flow, Hooks, Host, ParticleOptions, PlaceBlockEvent, RightClickBlockEvent, ServerTickEvent,
    ServerTickPhase, Text,
};

use super::diodes_signal as signal;
use super::display_link::{self, ID};
use crate::util;

/// Ticks between the dust that marks the selected block.
const MARK_RATE: i64 = 10;
/// The outline's colour in Create (`0xffcb74`).
const MARK_COLOUR: u32 = 0xFF_CB74;
const GREEN: u32 = 0x55FF55;
const RED: u32 = 0xFF5555;
const WHITE: u32 = 0xFFFFFF;

struct Select;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Select)
}

static TICKS: AtomicI64 = AtomicI64::new(0);

/// Each player's `CLICK_TO_LINK_DATA`: the dimension and position they selected.
static SELECTIONS: Mutex<BTreeMap<u128, (String, Pos)>> = Mutex::new(BTreeMap::new());

fn selections() -> std::sync::MutexGuard<'static, BTreeMap<u128, (String, Pos)>> {
    SELECTIONS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn holds_link(host: &dyn Host, uuid: u128) -> bool {
    host.player_equipment(uuid, EquipmentSlot::MainHand).is_some_and(|s| &*s.item == ID)
}

fn say(host: &mut dyn Host, uuid: u128, line: &str, colour: u32) {
    host.action_bar(uuid, Text::colored(line.to_owned(), colour));
}

impl Hooks for Select {
    /// `ClickToLinkBlockItem.useOn`.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR || !holds_link(&*host, player.uuid) {
            return Flow::Continue;
        }
        let pos = event.pos;
        let block = host.block(pos);
        // A link block takes the click itself (its screen), unless sneaking.
        if block == ID && !player.sneaking {
            return Flow::Continue;
        }
        if util::click_repeated(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let selected = selections().get(&player.uuid).cloned();
        let dimension = host.dimension().to_owned();
        match selected {
            Some(_) if player.sneaking => {
                selections().remove(&player.uuid);
                util::click_taken(&*host, player.uuid, pos);
                say(host, player.uuid, "Cleared position selection", WHITE);
                Flow::Cancel
            }
            None => {
                selections().insert(player.uuid, (dimension, pos));
                util::click_taken(&*host, player.uuid, pos);
                say(host, player.uuid, "Targeted position selected", WHITE);
                Flow::Cancel
            }
            Some((selected_dimension, selected_pos)) => {
                // `pos.relative(face, canBeReplaced ? 0 : 1)`: where the link would stand.
                let reach = i32::from(!signal::replaceable(&block));
                let placed = [
                    pos[0] + event.face[0] * reach,
                    pos[1] + event.face[1] * reach,
                    pos[2] + event.face[2] * reach,
                ];
                if !within_range(selected_pos, placed, selected_dimension == dimension) {
                    util::click_taken(&*host, player.uuid, pos);
                    say(host, player.uuid, "Targeted position is too far from here", RED);
                    return Flow::Cancel;
                }
                // Within reach: the click goes on and the link is placed (`place_block` aims it).
                Flow::Continue
            }
        }
    }

    /// The link placed takes the selection as its target (`TargetOffset`).
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID || host.block(event.pos) != ID {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let Some((_, selected)) = selections().remove(&player.uuid) else {
            return;
        };
        let pos = event.pos;
        let offset = [selected[0] - pos[0], selected[1] - pos[1], selected[2] - pos[2]];
        display_link::aim(host, pos, offset);
        say(host, player.uuid, "Successfully bound to targeted position", GREEN);
    }

    /// The selection ends when the link leaves the hand; until then the block is marked.
    fn server_tick(&self, host: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        let ServerTickPhase::ServerTickStart { world_running: true } = event.phase else {
            return;
        };
        let now = TICKS.fetch_add(1, Ordering::Relaxed);
        if now.rem_euclid(MARK_RATE) != 0 {
            return;
        }
        let players: Vec<(u128, Pos)> = selections().iter().map(|(who, (_, pos))| (*who, *pos)).collect();
        for (uuid, point) in players {
            if !holds_link(&*host, uuid) {
                selections().remove(&uuid);
                continue;
            }
            let [r, g, b] = [16, 8, 0].map(|shift| ((MARK_COLOUR >> shift) & 0xFF) as f32 / 255.0);
            let options = ParticleOptions::dust([r, g, b], 0.6);
            for corner in 0..8 {
                let at = [0, 1, 2].map(|axis| {
                    f64::from(point[axis]) + if corner >> axis & 1 == 0 { 0.02 } else { 0.98 }
                });
                host.send_particles_to(uuid, &options, None, at, [0.0; 3], 0.0, 1, false);
            }
        }
    }
}
