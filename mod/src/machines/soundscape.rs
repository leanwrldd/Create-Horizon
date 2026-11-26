//! The ambient machinery sound on the client (`SoundScapes`, `SoundScape`, `ContinuousSound`, `RepeatingSound`,
//! `KineticBlockEntity.tickAudio`): every running machine near the camera reports from its client tick, the
//! rules (`create_kinetics::rules::soundscape`) group the reports and decide what plays, and this module carries
//! the decisions out with `ClientHost::play_local_sound`. The block entities' reports come from `parts.rs`'s
//! `block_entity_client_tick`, the once-a-tick work from `lib.rs`'s `client_tick_post`.
//!
//! Known differences from Create, for the README:
//! - A looping sound has no setter for its position or volume. The loop's volume is faded to its new value over
//!   the update interval (`SoundRef::fade`), and the loop is stopped and started again when the group's mean
//!   position moves a block or more; Create moves and re-reads `ContinuousSound` every tick.
//! - `enableAmbientSounds` and `ambientVolumeCap` are fixed at Create's defaults (on, 0.1): there is no client
//!   config here.
//! - A resource reload (`SoundScapes.invalidateAll`) isn't heard; the groups end by themselves when their machines
//!   stop reporting, and leaving a world stops every loop.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use create_kinetics::rules::soundscape::{self as rules, Action, Group, PitchGroup, Scapes};
use crate::compat::{Hooks};
use crate::util;

/// A module of shared client code: no old-API hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

type LoopKey = (Group, PitchGroup, usize);

#[derive(Default)]
struct State {
    scapes: Scapes,
    /// `AnimationTickHolder.getTicks`: client ticks since the module started.
    ticks: i64,
    /// Where the camera was at the last client tick.
    camera: Option<[f64; 3]>,
    loops: BTreeMap<LoopKey, mod_api::SoundRef>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(State::default))
}

/// A kinetic machine's (or the crushing wheel controller's) client tick: what `tickAudio` reports, from the update
/// tag's `Speed` and `Busy`.
pub(crate) fn observe(event: &mod_api::BlockEntityClientTickEvent<'_>) {
    let entity = event.entity;
    let id = entity.id();
    if !rules::listens(id) {
        return;
    }
    let speed = util::float(&entity.nbt, "Speed").unwrap_or(0.0);
    let busy = util::boolean(&entity.nbt, "Busy");
    let reports = rules::reports(id, speed, busy);
    if reports.is_empty() {
        return;
    }
    let pos = event.pos;
    with(|state| {
        let Some(camera) = state.camera else { return };
        let block = camera.map(|v| v.floor() as i32);
        for (group, pitch) in reports {
            state.scapes.report(true, block, group, pos, pitch);
        }
    });
}

/// `SoundScapes.tick`, once a client tick: works out what every group plays and does it.
pub(crate) fn client_tick(host: &dyn mod_api::ClientHost) {
    let camera = host
        .camera_entity()
        .and_then(|entity| entity.position(host));
    let actions = with(|state| {
        let Some(camera) = camera else {
            // No world: every group ends.
            state.camera = None;
            return state.scapes.clear();
        };
        state.camera = Some(camera);
        state.ticks += 1;
        state.scapes.tick(state.ticks, camera, true, rules::DEFAULT_VOLUME_CAP)
    });
    for action in actions {
        carry_out(host, action);
    }
    if camera.is_none() {
        // The handles are stale anyway once a world is left.
        with(|state| state.loops.clear());
    }
}

fn play_of(event: &str, volume: f32, pitch: f32, at: [f64; 3]) -> mod_api::SoundPlay {
    mod_api::SoundPlay::at(event, mod_api::SoundSource::Ambient, at).with_volume(volume).with_pitch(pitch)
}

fn carry_out(host: &dyn mod_api::ClientHost, action: Action) {
    match action {
        Action::StartLoop { key, index, event, volume, pitch, at } | Action::MoveLoop { key, index, event, volume, pitch, at } => {
            let loop_key = (key.0, key.1, index);
            // A loop is stopped before another starts in its place.
            if let Some(old) = with(|state| state.loops.remove(&loop_key)) {
                old.stop(host);
            }
            let sound = play_of(event, volume, pitch, at).looping(0);
            if let Some(handle) = host.play_local_sound(&sound, 0) {
                with(|state| state.loops.insert(loop_key, handle));
            }
        }
        Action::FadeLoop { key, index, volume } => {
            if let Some(handle) = with(|state| state.loops.get(&(key.0, key.1, index)).copied()) {
                handle.fade(host, volume, rules::UPDATE_INTERVAL as u32);
            }
        }
        Action::StopLoop { key, index } => {
            if let Some(handle) = with(|state| state.loops.remove(&(key.0, key.1, index))) {
                handle.stop(host);
            }
        }
        Action::Once { event, volume, pitch, at } => {
            host.play_local_sound(&play_of(event, volume, pitch, at), 0);
        }
    }
}
