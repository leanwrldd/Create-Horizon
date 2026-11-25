//! The server's half of breaking a curve (`CurvedTrackDestroyPacket.applySettings`) and of the crack shown while it is held
//! (`destroyBlockProgress`): the client (`train_overlay.rs`) sends `create:curve_destroy` when the break progress of a picked curve
//! is full, or when a sneaking Wrench clicks it (`wrench`), and `create:curve_break` with the crack stage as the progress grows. The
//! decisions are `rules::trains::curve_interaction`.
//!
//! A destroy cuts the curve at both ends, and the graphs of the two blocks are made again. A Wrench gives the curve's tracks and girders
//! to the player (the rest dropped at their feet) with the `wrench_remove` sound; a break drops them along the curve
//! (`BezierConnection.spawnItems`, as the end of a curve does) unless the player is in creative. The break sound is the track's own
//! (`SoundType.METAL`: volume `(1 + 1) / 2`, pitch `.8`).
//!
//! Known differences: the destroy particles (`spawnDestroyParticles`) aren't sent, and `RULE_DOBLOCKDROPS` isn't read.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use create_kinetics::rules::trains::curve_interaction::{self as rules, CurveBreakStage, CurveDestroy};
use create_kinetics::rules::trains::track_block::{self, TRACK};
use crate::compat::{Hooks, Host, ItemStack, NativeServerPlayer, NetFromClientEvent, PlayerTickEvent};

use super::track as world;
use crate::util;

const BLOCKS: usize = 4;
const GIRDER: &str = "create:metal_girder";

struct Curves;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Curves)
}

enum Said {
    Destroy(CurveDestroy),
    Stage(CurveBreakStage),
}

static SAID: Mutex<Vec<(u128, Said)>> = Mutex::new(Vec::new());
static SAID_COUNT: AtomicUsize = AtomicUsize::new(0);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn eye(player: &NativeServerPlayer) -> [f64; 3] {
    [player.position[0], player.position[1] + rules::EYE_HEIGHT, player.position[2]]
}

fn stack(item: &str, count: u32) -> ItemStack {
    ItemStack { item: item.into(), count, damage: 0, extra: None }
}

/// `CurvedTrackDestroyPacket.applySettings`.
fn destroy(host: &mut dyn Host, player: &NativeServerPlayer, msg: CurveDestroy) {
    if !rules::may_destroy(player.game_mode, eye(player), msg.block) || host.block(msg.block) != TRACK {
        return;
    }
    let Some(bc) = world::connections_at(&*host, msg.block).into_iter().find(|c| c.key() == msg.curve) else { return };
    let dimension = host.dimension();

    world::remove_connection(host, msg.block, msg.curve);
    if host.block(msg.curve) == TRACK {
        world::remove_connection(host, msg.curve, msg.block);
    }
    // `TrackPropagator.onRailRemoved`: the two blocks' nodes are taken out of the graphs and made again.
    world::queue_retrack(dimension, msg.block);
    world::queue_retrack(dimension, msg.curve);

    let creative = player.game_mode == util::CREATIVE;
    if msg.wrench {
        let pitch = (util::random() % 1000) as f32 / 1000.0 * 0.5 + 0.5;
        host.play_sound("create:wrench_remove", util::centre(msg.sound_source), BLOCKS, 1.0, pitch);
        if !creative {
            for (girder, count) in rules::items_for_player(&bc) {
                let stack = stack(if girder { GIRDER } else { TRACK }, count);
                // `placeItemBackInInventory`: what doesn't fit lands at the player's feet.
                if let Some(rest) = host.give_stack(player.uuid, stack) {
                    util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
                }
            }
        }
    } else if !creative {
        let origin = bc.be_positions[0].map(f64::from);
        for (at, girders) in track_block::drop_points(&bc) {
            let at = [at[0] + origin[0], at[1] + origin[1], at[2] + origin[2]];
            util::spawn_item(host, at, &stack(TRACK, 1), [0.0; 3], 10);
            for _ in 0..girders {
                util::spawn_item(host, at, &stack(GIRDER, 1), [0.0; 3], 10);
            }
        }
    }
    host.play_sound("minecraft:block.metal.break", util::centre(msg.sound_source), BLOCKS, 1.0, 0.8);
    // The crack that led here is over.
    host.set_break_progress(player.entity_id, msg.block, -1);
}

/// `destroyBlockProgress(player id, pos, stage)`, as the client's progress asks.
fn stage(host: &mut dyn Host, player: &NativeServerPlayer, msg: CurveBreakStage) {
    if !rules::may_show_stage(player.game_mode, eye(player), msg.block) || host.block(msg.block) != TRACK {
        return;
    }
    host.set_break_progress(player.entity_id, msg.block, msg.stage);
}

impl Hooks for Curves {
    fn net_from_client(&self, _: &mut dyn Host, event: &mut NetFromClientEvent<'_>) {
        let said = match event.channel {
            rules::CHANNEL_DESTROY => CurveDestroy::decode(event.data).map(Said::Destroy),
            rules::CHANNEL_STAGE => CurveBreakStage::decode(event.data).map(Said::Stage),
            _ => None,
        };
        if let Some(said) = said {
            let mut queue = lock(&SAID);
            if queue.len() < 64 {
                queue.push((event.player, said));
            }
            SAID_COUNT.store(queue.len(), Ordering::Relaxed);
        }
    }

    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        if SAID_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        let mine: Vec<Said> = {
            let mut queue = lock(&SAID);
            let (mine, rest): (Vec<_>, Vec<_>) = queue.drain(..).partition(|(who, _)| *who == player.uuid);
            *queue = rest;
            SAID_COUNT.store(queue.len(), Ordering::Relaxed);
            mine.into_iter().map(|(_, said)| said).collect()
        };
        for said in mine {
            match said {
                Said::Destroy(msg) => destroy(host, &player, msg),
                Said::Stage(msg) => stage(host, &player, msg),
            }
        }
    }
}
