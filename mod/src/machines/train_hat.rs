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
//!   (`rules::trains::train_hat::TARGETS`), each with a head written down. Every type in Create's `train_hat_info` data
//!   (`rules::trains::train_hat_info::INFO`) too: a layer can't read the renderer's model as Create's does, so `client_setup` bakes the
//!   type's vanilla layer (`bake_layer("minecraft:cow#main")`, and `cow_baby#main` for a baby), turns it into the tree the rules
//!   walk (`ModelNode`) and keeps the head's mount; the layer looks its type up. Types whose layer has no head by that path
//!   (pufferfish and tropical fish have a model per variant) get no hat. Piglins take Horizon's own head pose, the rest turn the
//!   head's part with the entity's head angles. A baby's own model carries its size, so only the entity's `scale` is applied.
//! - **The Blaze Burner's hat** (`BlazeBurnerRenderer.renderShared`) is drawn by `burner::draw_head` on a carriage's conducting burner
//!   (`train_draw.rs` reads the `Hats` list `trains.rs` writes into the anchor, from `carriage_contraption::hat_burners`).
//! - The head slot isn't read (`getHatFor` returns nothing for a mob that wears a helmet), nor is `TrainHat` in a mob's persistent data.
//!
//! Type-checked and unit-tested (the geometry) only: not played.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use create_kinetics::rules::trains::train_hat::{self as rules, HeadPose, HeadSource, Mount, Op};
use crate::compat::{Hooks, Host, NetFromServerEvent, PlayerTickEvent};

use create_kinetics::rules::trains::train_hat_info as info;

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

// --- the heads of the types Create has data for ------------------------------------------------------------------------------------

/// What `client_setup` read of a type's model: the head of the adult and, where the game has one, the baby.
#[derive(Default)]
struct Baked {
    adult: Option<info::ModelMount>,
    baby: Option<info::ModelMount>,
}

static BAKED: LazyLock<Mutex<HashMap<String, Baked>>> = LazyLock::new(Mutex::default);

/// A baked part as the rules' tree: its pose now (as baked), the boxes its faces make and its children.
fn node_of(name: &str, part: &mod_api::ModelPart) -> info::ModelNode {
    let faces: Vec<[[f32; 3]; 4]> = part.polygons.iter().map(|p| p.vertices.map(|v| v.pos)).collect();
    info::ModelNode {
        name: name.to_owned(),
        pose: [part.x, part.y, part.z, part.x_rot, part.y_rot, part.z_rot, part.x_scale, part.y_scale, part.z_scale],
        cubes: info::cubes_of(&faces),
        children: part.children().map(|(n, c)| node_of(n, c)).collect(),
    }
}

fn mount_of(host: &dyn mod_api::ClientHost, layer: &str, entity_type: &str) -> Option<info::ModelMount> {
    let root = node_of("root", &host.bake_layer(layer)?);
    info::resolve(&root, entity_type, &info::info_for(entity_type))
}

/// `FMLClientSetupEvent`: the vanilla layers are baked and the heads found, once.
pub(crate) fn client_setup(host: &mut dyn mod_api::ClientHost) {
    let mut table = lock(&BAKED);
    for entity_type in info::types() {
        let adult = mount_of(&*host, &format!("{entity_type}#main"), entity_type);
        let baby = mount_of(&*host, &format!("{entity_type}_baby#main"), entity_type);
        table.insert(entity_type.to_owned(), Baked { adult, baby });
    }
}

/// The layer of a type whose head is in the table: `CreateHatArmorLayer` with `TrainHatInfo`.
struct ModelLayer {
    entity_type: String,
}

impl mod_api::EntityRenderLayer for ModelLayer {
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
        let living = entity.living;
        let baby = living.is_some_and(|l| l.is_baby);
        let (mount, scale) = {
            let table = lock(&BAKED);
            let Some(baked) = table.get(&self.entity_type) else { return Ok(()) };
            let scale = living.map_or(1.0, |l| l.scale);
            match (&baked.baby, baby) {
                (Some(own), true) => (own.clone(), scale),
                _ => {
                    let Some(adult) = baked.adult.clone() else { return Ok(()) };
                    (adult, if baby { scale * living.map_or(1.0, |l| l.age_scale) } else { scale })
                }
            }
        };
        let head = living.map_or([0.0; 2], |l| [f64::from(l.x_rot).to_radians(), f64::from(l.y_rot).to_radians()]);
        let posed = parts.and_then(|p| p.get(mod_api::HumanoidPart::Head)).map(|part| HeadPose {
            position: [f64::from(part.pose.x), f64::from(part.pose.y), f64::from(part.pose.z)],
            angles: [f64::from(part.pose.x_rot), f64::from(part.pose.y_rot), f64::from(part.pose.z_rot)],
        });
        draw.push();
        for op in info::mount_ops(&mount, head, posed.as_ref(), scale) {
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
    for entity_type in info::types().filter(|t| !rules::TARGETS.contains(t)) {
        let layer = Arc::new(ModelLayer { entity_type: entity_type.to_owned() });
        let _ = event.layers.add(mod_api::LayerTarget::Entity(entity_type.to_owned()), layer);
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
