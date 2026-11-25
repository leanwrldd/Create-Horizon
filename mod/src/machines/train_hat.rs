//! The Train Hat on a seated conductor (`CreateHatArmorLayer`, `EntityHats.getHatFor`). When a mob wears it is
//! `schedule_hand::wears_train_hat` (a rider of a seat in front of Train Controls, on a carriage with a schedule; a player's carriage
//! needs none); where it sits on a head is `rules::trains::train_hat`.
//!
//! - **Drawing on a mob**: Horizon's entity layers (`register_entity_layers`, `EntityRenderLayer::submit`) draw over a living
//!   entity of a type, in its own frame, after its model; humanoid models also hand over their head's pose (`PosedParts`). A layer
//!   has no host and the game's riders of a carriage are this mod's seat markers (which the client doesn't see), so the server tells
//!   each player which entities wear the hat (`create:train_hats`, entity ids, from `player_tick`) and the layer looks the frame's
//!   entity up in that list. The model is `create:entity/train_hat`, resolved with the other standalone models.
//! - **What is covered**: the zombie family, the skeletons, villagers, the wandering trader and the illagers, and players
//!   (`rules::trains::train_hat::TARGETS`). Create reads the head off any model's parts; a layer can't, so every other mob goes without.
//! - **The Blaze Burner's hat** (`BlazeBurnerRenderer`'s `drawHat`, a carriage's conducting burner) is not drawn: a carriage's burner is a
//!   block of the structure and the structure's draw (`train_draw.rs`) draws its blocks as models (`draw_block`), where the burner's head,
//!   goggles and hat are a block entity renderer's partial models. The rules for it (`burner_wears_hat`, `hat_placement`) are in
//!   `schedule_hand.rs`; the hat would have to be drawn there once the burner's head is.
//! - The head slot isn't read (`getHatFor` returns nothing for a mob that wears a helmet), nor is `TrainHat` in a mob's persistent data.
//!
//! Type-checked and unit-tested (the geometry) only: not played.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use create_kinetics::rules::trains::train_hat::{self as rules, HeadPose, HeadSource, Mount, Op};
use crate::compat::{Hooks, Host, NetFromServerEvent, PlayerTickEvent};

use super::trains;
use crate::util;

pub const CHANNEL: &str = "create:train_hats";
/// How often the server works out who wears the hat (ticks), and how often it repeats an unchanged list.
const EVERY: i64 = 10;
const REPEAT: i64 = 200;

struct Hats;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Hats)
}

/// The entities that wear the hat, as the client last heard.
static WEARERS: LazyLock<Mutex<HashSet<u32>>> = LazyLock::new(Mutex::default);
/// What each player was last told, and when.
static SENT: LazyLock<Mutex<HashMap<u128, (Vec<u32>, i64)>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn encode(ids: &[u32]) -> Vec<u8> {
    let mut out = (ids.len().min(u16::MAX as usize) as u16).to_be_bytes().to_vec();
    for id in ids.iter().take(u16::MAX as usize) {
        out.extend_from_slice(&id.to_be_bytes());
    }
    out
}

fn decode(data: &[u8]) -> Option<Vec<u32>> {
    let (count, rest) = data.split_at_checked(2)?;
    let count = usize::from(u16::from_be_bytes([count[0], count[1]]));
    if rest.len() != count * 4 {
        return None;
    }
    Some(rest.chunks_exact(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect())
}

// --- the layer ---------------------------------------------------------------------------------------------------------------------

struct HatLayer {
    mount: Mount,
}

impl mod_api::EntityRenderLayer for HatLayer {
    fn submit(
        &self,
        entity: &mod_api::EntityRenderState,
        parts: Option<&mod_api::PosedParts>,
        draw: &mut dyn mod_api::BlockEntityDrawContext,
    ) -> Result<(), String> {
        if !lock(&WEARERS).contains(&entity.id) {
            return Ok(());
        }
        let Some(model) = crate::compat::adapter::model(rules::MODEL) else { return Ok(()) };
        let head = match self.mount.source {
            HeadSource::Posed => {
                let Some(part) = parts.and_then(|p| p.get(mod_api::HumanoidPart::Head)) else { return Ok(()) };
                if !part.visible {
                    return Ok(());
                }
                let pose = part.pose;
                HeadPose {
                    position: [f64::from(pose.x), f64::from(pose.y), f64::from(pose.z)],
                    angles: [f64::from(pose.x_rot), f64::from(pose.y_rot), f64::from(pose.z_rot)],
                }
            }
            HeadSource::Free => {
                let (yaw, pitch) = entity.living.map_or((0.0, 0.0), |l| (l.y_rot, l.x_rot));
                HeadPose { position: [0.0; 3], angles: [f64::from(pitch).to_radians(), f64::from(yaw).to_radians(), 0.0] }
            }
        };
        let size = entity.living.map_or(1.0, |l| l.scale * l.age_scale);
        draw.push();
        for op in rules::hat_ops(&self.mount, &head, size) {
            match op {
                Op::Translate(v) => draw.translate(v.map(|x| x as f32)),
                Op::Scale(v) => draw.scale(v.map(|x| x as f32)),
                Op::Rotate(axis, degrees) => draw.rotate_about(axis, degrees, [0.0; 3]),
            }
        }
        let drawn = draw.model(model, rules::TINT);
        draw.pop();
        drawn.map_err(|e| format!("train hat: {e:?}"))
    }
}

/// `CreateHatArmorLayer.registerOnAll`: the layer on every type that has a known head, and on both player models.
pub(crate) fn register_entity_layers(event: &mut mod_api::RegisterEntityLayersEvent<'_>) {
    for target in rules::TARGETS {
        let Some(mount) = rules::mount_for(target) else { continue };
        let _ = event.layers.add(mod_api::LayerTarget::Entity((*target).to_owned()), Arc::new(HatLayer { mount }));
    }
    for model in [mod_api::PlayerModel::Wide, mod_api::PlayerModel::Slim] {
        let _ = event.layers.add(mod_api::LayerTarget::Player(model), Arc::new(HatLayer { mount: rules::PLAYER }));
    }
}

// --- who wears it --------------------------------------------------------------------------------------------------------------------

impl Hooks for Hats {
    /// The server tells the player who wears the hat in the player's dimension.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let now = util::game_time(&*host);
        if now % EVERY != 0 {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        let mut ids = trains::hat_wearers(&*host, host.dimension());
        ids.sort_unstable();
        let fresh = {
            let sent = lock(&SENT);
            match sent.get(&player.uuid) {
                Some((last, at)) => *last != ids || now - *at >= REPEAT || now < *at,
                None => !ids.is_empty(),
            }
        };
        if fresh && host.net_send_to_player(player.uuid, CHANNEL, &encode(&ids)).is_ok() {
            lock(&SENT).insert(player.uuid, (ids, now));
        }
    }

    fn net_from_server(&self, _: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        if event.channel == CHANNEL
            && let Some(ids) = decode(event.data)
        {
            *lock(&WEARERS) = ids.into_iter().collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_survives_the_wire() {
        let ids = [3, 70_000, 9];
        assert_eq!(decode(&encode(&ids)), Some(vec![3, 70_000, 9]));
        assert_eq!(decode(&encode(&[])), Some(vec![]));
        assert_eq!(decode(&[0, 2, 0, 0, 0, 1]), None);
        assert_eq!(decode(&[0]), None);
    }
}
