//! Create's ambient machinery sound (`SoundScapes`, `SoundScape`, `ContinuousSound`, `RepeatingSound`, and the
//! `tickAudio` of `KineticBlockEntity`, `MillstoneBlockEntity` and `CrushingWheelControllerBlockEntity`): every
//! running machine near the camera reports itself each client tick; reports are grouped by ambience group and pitch
//! group; each group makes ONE sound at the mean position of its machines, as loud as there are machines near.
//! No game types: the glue is `machines/soundscape.rs`.
//!
//! What a scape plays:
//! - `KINETIC`: a looping `minecraft:entity.minecart.inside` at 0.25;
//! - `COG`: a looping `create:cogs` at 1.5;
//! - `CRUSHING`: `crushing_1` (volume 1.545, pitch 0.75) every tick, `crushing_2` (0.425, 0.75) every 2 ticks and
//!   `crushing_3` (2, 1.75) every 2 ticks, played again at each cadence;
//! - `MILLING`: `crushing_1` every tick and `crushing_2` every 2 ticks, as above without the third.
//!
//! All at the group's shared pitch (the first reporter's) times the sound's relative pitch.
//!
//! Known differences from Create, for the README:
//! - A looping sound can't move or change volume each tick (Horizon's `play_local_sound` loops, and can be faded
//!   or stopped, but not moved): the volume is faded to its new value every update interval and the loop is
//!   started again when the mean position has moved a block or more ([`MOVE_RESTART`]).
//! - `enableAmbientSounds` and `ambientVolumeCap` (a client config) are fixed at Create's defaults (on, 0.1).
//! - Counters are cleared after the group's sounds are worked out, not before, so a count is the machines that
//!   reported over the last update interval.

use std::collections::{BTreeMap, BTreeSet};

use super::sound_entries;

pub type Pos = [i32; 3];

/// `SoundScapes.MAX_AMBIENT_SOURCE_DISTANCE`.
pub const MAX_DISTANCE: i32 = 16;
/// `SoundScapes.UPDATE_INTERVAL`.
pub const UPDATE_INTERVAL: i64 = 5;
/// `SoundScapes.SOUND_VOLUME_ARG_MAX`.
pub const VOLUME_ARG_MAX: f32 = 15.0;
/// `CClient.ambientVolumeCap` default.
pub const DEFAULT_VOLUME_CAP: f32 = 0.1;
/// A loop whose mean position moved this far (blocks) is started again at the new spot.
pub const MOVE_RESTART: f64 = 1.0;
/// A loop's volume is faded to a new value only when it differs by more than this.
pub const VOLUME_STEP: f32 = 0.002;

/// `SoundScapes.AmbienceGroup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Kinetic,
    Cog,
    Crushing,
    Milling,
}

/// `SoundScapes.PitchGroup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PitchGroup {
    VeryLow,
    Low,
    Normal,
    High,
    VeryHigh,
}

/// `SoundScapes.getGroupFromPitch`.
pub fn pitch_group(pitch: f32) -> PitchGroup {
    if pitch < 0.70 {
        PitchGroup::VeryLow
    } else if pitch < 0.90 {
        PitchGroup::Low
    } else if pitch < 1.10 {
        PitchGroup::Normal
    } else if pitch < 1.30 {
        PitchGroup::High
    } else {
        PitchGroup::VeryHigh
    }
}

/// The pitch every `tickAudio` computes: `Mth.clamp(speed / 256 + .45, .85, 1)`.
pub fn speed_pitch(speed: f32) -> f32 {
    ((speed.abs() / 256.0) + 0.45).clamp(0.85, 1.0)
}

/// A looping sound of a scape: its event, volume and pitch relative to the scape's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Continuous {
    pub event: &'static str,
    pub volume: f32,
    pub pitch: f32,
}

/// A sound a scape plays again every `delay` ticks (`RepeatingSound`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Repeating {
    pub event: &'static str,
    pub volume: f32,
    pub pitch: f32,
    pub delay: i64,
}

/// What a group's `SoundScape` is made of (`SoundScapes.kinetic`, `cogwheel`, `crushing`, `milling`).
pub fn continuous(group: Group) -> &'static [Continuous] {
    match group {
        Group::Kinetic => &[Continuous { event: "minecraft:entity.minecart.inside", volume: 0.25, pitch: 1.0 }],
        Group::Cog => &[Continuous { event: "create:cogs", volume: 1.5, pitch: 1.0 }],
        Group::Crushing | Group::Milling => &[],
    }
}

pub fn repeating(group: Group) -> &'static [Repeating] {
    match group {
        Group::Crushing => &[
            Repeating { event: "create:crushing_1", volume: 1.545, pitch: 0.75, delay: 1 },
            Repeating { event: "create:crushing_2", volume: 0.425, pitch: 0.75, delay: 2 },
            Repeating { event: "create:crushing_3", volume: 2.0, pitch: 1.75, delay: 2 },
        ],
        Group::Milling => &[
            Repeating { event: "create:crushing_1", volume: 1.545, pitch: 0.75, delay: 1 },
            Repeating { event: "create:crushing_2", volume: 0.425, pitch: 0.75, delay: 2 },
        ],
        Group::Kinetic | Group::Cog => &[],
    }
}

/// `SoundScapes.outOfRange`: the camera's block (`blockPosition`) is not within 16 of the machine
/// (`Vec3i.closerThan`: squared distance under the squared range).
pub fn out_of_range(camera_block: Pos, pos: Pos) -> bool {
    let d = |i: usize| i64::from(camera_block[i]) - i64::from(pos[i]);
    let range = i64::from(MAX_DISTANCE);
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2) >= range * range
}

/// `SoundScape.getVolume`: `clamp(count / (15 * 10), 0.025, cap) * lerp(distance / 16, 2, 0)`. A distance past the
/// range (the mean can sit a little further than the machines) is silent, not negative.
pub fn volume(count: usize, distance: f64, cap: f32) -> f32 {
    let multiplier = (2.0 + (distance / f64::from(MAX_DISTANCE)) * (0.0 - 2.0)).max(0.0) as f32;
    // `Mth.clamp(float, min, max)`: below the minimum is the minimum, whatever the maximum.
    let v = count as f32 / (VOLUME_ARG_MAX * 10.0);
    let clamped = if v < 0.025 {
        0.025
    } else if v > cap {
        cap
    } else {
        v
    };
    clamped * multiplier
}

/// `SoundScape.determineMeanPos`: the centre of every machine, averaged (the origin for none).
pub fn mean_pos(positions: &BTreeSet<Pos>) -> [f64; 3] {
    if positions.is_empty() {
        return [0.0; 3];
    }
    let mut sum = [0.0f64; 3];
    for p in positions {
        for i in 0..3 {
            sum[i] += f64::from(p[i]) + 0.5;
        }
    }
    let n = positions.len() as f64;
    [sum[0] / n, sum[1] / n, sum[2] / n]
}

// --- what each machine reports ------------------------------------------------------------------------------------

/// `KineticBlockEntity.isNoisy`: false for `SimpleKineticBlockEntity` (shafts, cogwheels and their encased forms),
/// `GearboxBlockEntity`, `GantryShaftBlockEntity` and `FlapDisplayBlockEntity`.
pub fn is_noisy(block: &str) -> bool {
    let block = block.strip_prefix("create:").unwrap_or(block);
    !matches!(
        block,
        "shaft"
            | "cogwheel"
            | "large_cogwheel"
            | "andesite_encased_shaft"
            | "brass_encased_shaft"
            | "andesite_encased_cogwheel"
            | "brass_encased_cogwheel"
            | "andesite_encased_large_cogwheel"
            | "brass_encased_large_cogwheel"
            | "encased_chain_drive"
            | "gearbox"
            | "vertical_gearbox"
            | "gantry_shaft"
            | "display_board"
    )
}

/// `ICogWheel.isSmallCog` or `isLargeCog`, or a `GearboxBlock`: the blocks that make the cog rumble.
pub fn is_cog(block: &str) -> bool {
    let block = block.strip_prefix("create:").unwrap_or(block);
    matches!(
        block,
        // `ICogWheel`: small
        "cogwheel"
            | "andesite_encased_cogwheel"
            | "brass_encased_cogwheel"
            | "mechanical_crafter"
            | "mechanical_arm"
            | "millstone"
            | "mechanical_mixer"
            | "mechanical_pump"
            | "display_board"
            // `ICogWheel`: large
            | "large_cogwheel"
            | "andesite_encased_large_cogwheel"
            | "brass_encased_large_cogwheel"
            // `GearboxBlock`
            | "gearbox"
            | "vertical_gearbox"
    )
}

/// The kinetic block entities (`extends KineticBlockEntity`), by block name: the ones whose client tick reports.
/// A dyed valve handle is any `<colour>_valve_handle`.
pub fn is_kinetic(block: &str) -> bool {
    let block = block.strip_prefix("create:").unwrap_or(block);
    block.ends_with("_valve_handle")
        || matches!(
            block,
            "shaft"
                | "cogwheel"
                | "large_cogwheel"
                | "creative_motor"
                | "gearbox"
                | "vertical_gearbox"
                | "andesite_encased_cogwheel"
                | "brass_encased_cogwheel"
                | "andesite_encased_large_cogwheel"
                | "brass_encased_large_cogwheel"
                | "andesite_encased_shaft"
                | "brass_encased_shaft"
                | "adjustable_chain_gearshift"
                | "encased_fan"
                | "clutch"
                | "gearshift"
                | "turntable"
                | "hand_crank"
                | "cuckoo_clock"
                | "mysterious_cuckoo_clock"
                | "gantry_shaft"
                | "gantry_carriage"
                | "chain_conveyor"
                | "mechanical_pump"
                | "fluid_valve"
                | "hose_pulley"
                | "belt"
                | "mechanical_arm"
                | "mechanical_piston"
                | "sticky_mechanical_piston"
                | "windmill_bearing"
                | "mechanical_bearing"
                | "clockwork_bearing"
                | "rope_pulley"
                | "elevator_pulley"
                | "mechanical_drill"
                | "mechanical_saw"
                | "powered_shaft"
                | "flywheel"
                | "millstone"
                | "crushing_wheel"
                | "water_wheel"
                | "large_water_wheel"
                | "mechanical_press"
                | "mechanical_mixer"
                | "deployer"
                | "mechanical_crafter"
                | "sequenced_gearshift"
                | "rotation_speed_controller"
                | "speedometer"
                | "stressometer"
                | "weighted_ejector"
                | "copper_backtank"
                | "netherite_backtank"
                | "display_board"
        )
}

/// Which block entities tick on the client for sound: the kinetic ones and the crushing wheel's controller.
pub fn listens(block: &str) -> bool {
    let name = block.strip_prefix("create:").unwrap_or(block);
    name == "crushing_wheel_controller" || is_kinetic(name)
}

/// One machine's report: its group and its pitch.
pub type Report = (Group, f32);

/// `KineticBlockEntity.tickAudio`: a spinning block reports `KINETIC` when it is noisy, and `COG` when it is a
/// cogwheel or gearbox, at the pitch its speed gives. A stopped block reports nothing.
pub fn kinetic_reports(block: &str, speed: f32) -> Vec<Report> {
    if speed.abs() == 0.0 {
        return Vec::new();
    }
    let pitch = speed_pitch(speed);
    let mut out = Vec::new();
    if is_noisy(block) {
        out.push((Group::Kinetic, pitch));
    }
    if is_cog(block) {
        out.push((Group::Cog, pitch));
    }
    out
}

/// `MillstoneBlockEntity.tickAudio`: what the kinetic one reports, and `MILLING` while it turns with something in
/// its input slot.
pub fn millstone_reports(speed: f32, has_input: bool) -> Vec<Report> {
    let mut out = kinetic_reports("millstone", speed);
    if speed != 0.0 && has_input {
        out.push((Group::Milling, speed_pitch(speed)));
    }
    out
}

/// `CrushingWheelControllerBlockEntity.tickAudio` (called from its tick while it is occupied and the wheels
/// turn): `CRUSHING` at the crushing speed's pitch while an entity or an item is in it.
pub fn crushing_report(crushing_speed: f32, occupied: bool) -> Option<Report> {
    if crushing_speed == 0.0 || !occupied {
        return None;
    }
    Some((Group::Crushing, speed_pitch(crushing_speed)))
}

/// What a block entity of the client reports this tick, from its block name and the numbers the update tag carries.
/// `speed` is the kinetic speed (`Speed`), `busy` is the controller's "something inside" or the millstone's "input
/// not empty" (`Busy`).
pub fn reports(block: &str, speed: f32, busy: bool) -> Vec<Report> {
    let name = block.strip_prefix("create:").unwrap_or(block);
    match name {
        "crushing_wheel_controller" => crushing_report(speed, busy).into_iter().collect(),
        "millstone" => millstone_reports(speed, busy),
        _ if is_kinetic(name) => kinetic_reports(name, speed),
        _ => Vec::new(),
    }
}

// --- the scapes -------------------------------------------------------------------------------------------------

type Key = (Group, PitchGroup);

/// A sound to start or change, for the glue to carry out.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Start a looping sound (`ContinuousSound` played): `index` is its place in [`continuous`].
    StartLoop { key: Key, index: usize, event: &'static str, volume: f32, pitch: f32, at: [f64; 3] },
    /// Fade a running loop to a new volume.
    FadeLoop { key: Key, index: usize, volume: f32 },
    /// The loop's mean position moved: start it again at `at` (the glue stops the old one).
    MoveLoop { key: Key, index: usize, event: &'static str, volume: f32, pitch: f32, at: [f64; 3] },
    /// Stop a loop for good (`ContinuousSound.remove`).
    StopLoop { key: Key, index: usize },
    /// One play of a repeating sound (`RepeatingSound.tick`: `playLocalSound`).
    Once { event: &'static str, volume: f32, pitch: f32, at: [f64; 3] },
}

#[derive(Clone, Debug)]
struct Scape {
    /// The first reporter's pitch: `SoundScape.pitch`.
    pitch: f32,
    /// Per loop: where and how loud it was started or last set, `None` before it starts.
    loops: Vec<Option<([f64; 3], f32)>>,
}

/// `SoundScapes`' statics: the machines counted in this interval and the scapes that exist.
#[derive(Clone, Debug, Default)]
pub struct Scapes {
    counter: BTreeMap<Key, BTreeSet<Pos>>,
    active: BTreeMap<Key, Scape>,
}

impl Scapes {
    /// `SoundScapes.play`: a machine at `pos` reports. Nothing when `enabled` is off or it is out of range of the
    /// camera's block; otherwise the machine is counted and its scape made if there is none.
    pub fn report(&mut self, enabled: bool, camera_block: Pos, group: Group, pos: Pos, pitch: f32) {
        if !enabled || out_of_range(camera_block, pos) {
            return;
        }
        let key = (group, pitch_group(pitch));
        self.counter.entry(key).or_default().insert(pos);
        self.active.entry(key).or_insert_with(|| Scape { pitch, loops: vec![None; continuous(group).len()] });
    }

    /// The machines counted for a scape.
    pub fn count(&self, group: Group, pitch_group: PitchGroup) -> usize {
        self.counter.get(&(group, pitch_group)).map_or(0, BTreeSet::len)
    }

    /// Scapes that exist.
    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// `SoundScapes.invalidateAll`: every scape ends.
    pub fn clear(&mut self) -> Vec<Action> {
        self.counter.clear();
        let mut out = Vec::new();
        for (key, scape) in std::mem::take(&mut self.active) {
            stop_all(&mut out, key, &scape);
        }
        out
    }

    /// `SoundScapes.tick`, once a client tick (`ticks` is `AnimationTickHolder.getTicks`): the repeating sounds play
    /// at their cadence, the loops follow the volume; every update interval the scapes with no machines (or all of
    /// them when disabled) end and the counters start again.
    pub fn tick(&mut self, ticks: i64, camera: [f64; 3], enabled: bool, cap: f32) -> Vec<Action> {
        let mut out = Vec::new();
        let update = ticks.rem_euclid(UPDATE_INTERVAL) == 0;
        let keys: Vec<Key> = self.active.keys().copied().collect();
        for key in keys {
            let empty = BTreeSet::new();
            let positions = self.counter.get(&key).unwrap_or(&empty);
            let count = positions.len();
            if count == 0 || !enabled {
                // Ends at the next update below; silent until then (its volume is the floor).
                continue;
            }
            let mean = mean_pos(positions);
            let distance = ((camera[0] - mean[0]).powi(2) + (camera[1] - mean[1]).powi(2) + (camera[2] - mean[2]).powi(2)).sqrt();
            let volume = volume(count, distance, cap);
            let scape = self.active.get_mut(&key).expect("key came from the map");
            for (i, sound) in repeating(key.0).iter().enumerate() {
                let _ = i;
                if ticks.rem_euclid(sound.delay.max(1)) == 0 {
                    out.push(Action::Once {
                        event: sound.event,
                        volume: volume * sound.volume,
                        pitch: scape.pitch * sound.pitch,
                        at: mean,
                    });
                }
            }
            for (index, sound) in continuous(key.0).iter().enumerate() {
                let target = volume * sound.volume;
                let pitch = scape.pitch * sound.pitch;
                match scape.loops[index] {
                    None => {
                        scape.loops[index] = Some((mean, target));
                        out.push(Action::StartLoop { key, index, event: sound.event, volume: target, pitch, at: mean });
                    }
                    Some((at, last)) if update => {
                        let moved = ((at[0] - mean[0]).powi(2) + (at[1] - mean[1]).powi(2) + (at[2] - mean[2]).powi(2)).sqrt();
                        if moved >= MOVE_RESTART {
                            scape.loops[index] = Some((mean, target));
                            out.push(Action::MoveLoop { key, index, event: sound.event, volume: target, pitch, at: mean });
                        } else if (target - last).abs() > VOLUME_STEP {
                            scape.loops[index] = Some((at, target));
                            out.push(Action::FadeLoop { key, index, volume: target });
                        }
                    }
                    Some(_) => {}
                }
            }
        }
        if update {
            let dead: Vec<Key> = self
                .active
                .keys()
                .copied()
                .filter(|key| !enabled || self.count(key.0, key.1) == 0)
                .collect();
            for key in dead {
                if let Some(scape) = self.active.remove(&key) {
                    stop_all(&mut out, key, &scape);
                }
            }
            self.counter.values_mut().for_each(BTreeSet::clear);
        }
        out
    }
}

fn stop_all(out: &mut Vec<Action>, key: Key, scape: &Scape) {
    for (index, started) in scape.loops.iter().enumerate() {
        if started.is_some() {
            out.push(Action::StopLoop { key, index });
        }
    }
}

/// The create entries a scape's `create:` events name must be real ones: a test hook for the glue.
pub fn names_known_events() -> bool {
    [Group::Cog, Group::Crushing, Group::Milling]
        .iter()
        .flat_map(|&g| continuous(g).iter().map(|c| c.event).chain(repeating(g).iter().map(|r| r.event)))
        .filter_map(|event| event.strip_prefix("create:"))
        .all(|name| sound_entries::entry(name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMERA: Pos = [0, 64, 0];
    const EYE: [f64; 3] = [0.5, 64.0, 0.5];

    #[test]
    fn pitch_groups_split_at_create_thresholds() {
        assert_eq!(pitch_group(0.69), PitchGroup::VeryLow);
        assert_eq!(pitch_group(0.70), PitchGroup::Low);
        assert_eq!(pitch_group(0.89), PitchGroup::Low);
        assert_eq!(pitch_group(0.90), PitchGroup::Normal);
        assert_eq!(pitch_group(1.09), PitchGroup::Normal);
        assert_eq!(pitch_group(1.10), PitchGroup::High);
        assert_eq!(pitch_group(1.29), PitchGroup::High);
        assert_eq!(pitch_group(1.30), PitchGroup::VeryHigh);
    }

    #[test]
    fn speed_pitch_is_clamped_to_85_and_1() {
        assert_eq!(speed_pitch(16.0), 0.85);
        assert_eq!(speed_pitch(-16.0), 0.85);
        assert!((speed_pitch(128.0) - 0.95).abs() < 1e-6);
        assert_eq!(speed_pitch(256.0), 1.0);
        assert_eq!(speed_pitch(10000.0), 1.0);
    }

    #[test]
    fn volume_follows_the_count_the_cap_and_the_distance() {
        // 1 machine: 1/150 is under the floor 0.025; right next to the camera the multiplier is 2.
        assert!((volume(1, 0.0, 0.1) - 0.05).abs() < 1e-6);
        // 8 machines: 8/150 = 0.0533
        assert!((volume(8, 0.0, 0.1) - 0.10667).abs() < 1e-4);
        // many machines hit the cap.
        assert!((volume(1000, 0.0, 0.1) - 0.2).abs() < 1e-6);
        // half the range halves the multiplier (lerp(0.5, 2, 0) = 1).
        assert!((volume(1000, 8.0, 0.1) - 0.1).abs() < 1e-6);
        // the range's end and beyond are silent.
        assert_eq!(volume(1000, 16.0, 0.1), 0.0);
        assert_eq!(volume(1000, 30.0, 0.1), 0.0);
        // a cap under the floor still gives the floor (`Mth.clamp`).
        assert!((volume(1, 0.0, 0.01) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn the_mean_position_is_the_average_of_the_block_centres() {
        let set: BTreeSet<Pos> = [[0, 0, 0], [2, 0, 4]].into_iter().collect();
        assert_eq!(mean_pos(&set), [1.5, 0.5, 2.5]);
        assert_eq!(mean_pos(&BTreeSet::new()), [0.0; 3]);
    }

    #[test]
    fn range_is_the_camera_block_within_16() {
        assert!(!out_of_range([0, 0, 0], [15, 0, 0]));
        assert!(out_of_range([0, 0, 0], [16, 0, 0]));
        assert!(!out_of_range([0, 0, 0], [9, 9, 9])); // sqrt(243) = 15.6
        assert!(out_of_range([0, 0, 0], [10, 10, 10])); // sqrt(300) = 17.3
    }

    #[test]
    fn quiet_blocks_do_not_make_the_kinetic_noise_but_cogs_and_gearboxes_rumble() {
        assert!(kinetic_reports("create:shaft", 64.0).is_empty());
        assert_eq!(kinetic_reports("create:cogwheel", 64.0), vec![(Group::Cog, speed_pitch(64.0))]);
        assert_eq!(kinetic_reports("create:large_cogwheel", 64.0), vec![(Group::Cog, speed_pitch(64.0))]);
        assert_eq!(kinetic_reports("create:gearbox", 64.0), vec![(Group::Cog, speed_pitch(64.0))]);
        assert_eq!(kinetic_reports("create:mechanical_press", 64.0), vec![(Group::Kinetic, speed_pitch(64.0))]);
        // an ICogWheel that is also noisy makes both
        assert_eq!(
            kinetic_reports("create:mechanical_mixer", 64.0),
            vec![(Group::Kinetic, speed_pitch(64.0)), (Group::Cog, speed_pitch(64.0))]
        );
        // a display board is a cog but not noisy
        assert_eq!(kinetic_reports("create:display_board", 64.0), vec![(Group::Cog, speed_pitch(64.0))]);
        assert!(kinetic_reports("create:mechanical_press", 0.0).is_empty());
        assert!(kinetic_reports("create:gantry_shaft", 64.0).is_empty());
    }

    #[test]
    fn the_millstone_mills_only_with_something_in_it() {
        let idle = millstone_reports(64.0, false);
        assert!(idle.iter().all(|(g, _)| *g != Group::Milling));
        assert_eq!(idle.len(), 2); // kinetic and cog
        let busy = millstone_reports(64.0, true);
        assert!(busy.contains(&(Group::Milling, speed_pitch(64.0))));
        assert!(millstone_reports(0.0, true).is_empty());
    }

    #[test]
    fn the_crushing_wheel_controller_needs_speed_and_something_inside() {
        assert_eq!(crushing_report(16.0, true), Some((Group::Crushing, 0.85)));
        assert_eq!(crushing_report(0.0, true), None);
        assert_eq!(crushing_report(16.0, false), None);
        assert_eq!(reports("create:crushing_wheel_controller", 16.0, true), vec![(Group::Crushing, 0.85)]);
        assert!(reports("create:crushing_wheel_controller", 16.0, false).is_empty());
    }

    #[test]
    fn which_block_entities_listen() {
        assert!(listens("create:cogwheel"));
        assert!(listens("create:red_valve_handle"));
        assert!(listens("create:crushing_wheel_controller"));
        assert!(!listens("create:depot"));
    }

    #[test]
    fn a_report_makes_a_scape_that_starts_a_loop_and_ends_when_the_machines_stop() {
        let mut scapes = Scapes::default();
        scapes.report(true, CAMERA, Group::Cog, [3, 64, 0], 0.85);
        assert_eq!(scapes.len(), 1);
        let actions = scapes.tick(1, EYE, true, DEFAULT_VOLUME_CAP);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::StartLoop { event, pitch, at, volume, .. } => {
                assert_eq!(*event, "create:cogs");
                assert!((pitch - 0.85).abs() < 1e-6);
                assert_eq!(*at, [3.5, 64.5, 0.5]);
                // 1 machine, 3 blocks away: floor 0.025 * lerp(3/16) * relative 1.5
                let distance = 3.0f64.hypot(0.5);
                let expected = volume_of(1, distance) * 1.5;
                assert!((volume - expected).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        // Tick 5: the interval ends; the machine reported in it, so the scape stays and the counter resets.
        assert!(scapes.tick(5, EYE, true, DEFAULT_VOLUME_CAP).iter().all(|a| !matches!(a, Action::StopLoop { .. })));
        assert_eq!(scapes.len(), 1);
        // No report in the next interval: it ends at tick 10.
        let end = scapes.tick(10, EYE, true, DEFAULT_VOLUME_CAP);
        assert!(end.contains(&Action::StopLoop { key: (Group::Cog, PitchGroup::Low), index: 0 }));
        assert!(scapes.is_empty());
    }

    fn volume_of(count: usize, distance: f64) -> f32 {
        volume(count, distance, DEFAULT_VOLUME_CAP)
    }

    #[test]
    fn machines_of_one_pitch_group_share_one_scape_and_pitch_groups_are_separate() {
        let mut scapes = Scapes::default();
        scapes.report(true, CAMERA, Group::Kinetic, [1, 64, 0], 0.85);
        scapes.report(true, CAMERA, Group::Kinetic, [2, 64, 0], 0.86);
        scapes.report(true, CAMERA, Group::Kinetic, [3, 64, 0], 1.0);
        assert_eq!(scapes.len(), 2);
        assert_eq!(scapes.count(Group::Kinetic, PitchGroup::Low), 2);
        assert_eq!(scapes.count(Group::Kinetic, PitchGroup::Normal), 1);
        // the scape keeps the first reporter's pitch
        let actions = scapes.tick(1, EYE, true, DEFAULT_VOLUME_CAP);
        let low = actions.iter().find_map(|a| match a {
            Action::StartLoop { key: (_, PitchGroup::Low), pitch, .. } => Some(*pitch),
            _ => None,
        });
        assert_eq!(low, Some(0.85));
    }

    #[test]
    fn a_machine_out_of_range_or_a_disabled_scape_is_not_counted() {
        let mut scapes = Scapes::default();
        scapes.report(true, CAMERA, Group::Cog, [40, 64, 0], 0.9);
        scapes.report(false, CAMERA, Group::Cog, [1, 64, 0], 0.9);
        assert!(scapes.is_empty());
    }

    #[test]
    fn repeating_sounds_play_at_their_cadence() {
        let mut scapes = Scapes::default();
        scapes.report(true, CAMERA, Group::Crushing, [2, 64, 0], 0.85);
        let odd = scapes.tick(1, EYE, true, DEFAULT_VOLUME_CAP);
        let events: Vec<_> = odd.iter().filter_map(|a| if let Action::Once { event, .. } = a { Some(*event) } else { None }).collect();
        assert_eq!(events, vec!["create:crushing_1"]);
        let even = scapes.tick(2, EYE, true, DEFAULT_VOLUME_CAP);
        let events: Vec<_> = even.iter().filter_map(|a| if let Action::Once { event, .. } = a { Some(*event) } else { None }).collect();
        assert_eq!(events, vec!["create:crushing_1", "create:crushing_2", "create:crushing_3"]);
        // crushing_3 is at pitch 1.75 relative, volume 2 relative
        let third = even
            .iter()
            .find_map(|a| match a {
                Action::Once { event: "create:crushing_3", volume, pitch, .. } => Some((*volume, *pitch)),
                _ => None,
            })
            .expect("crushing_3");
        assert!((third.1 - 0.85 * 1.75).abs() < 1e-6);
        assert!(third.0 > 0.0);
        // The milling scape has no third sound.
        let mut milling = Scapes::default();
        milling.report(true, CAMERA, Group::Milling, [2, 64, 0], 0.85);
        let even = milling.tick(2, EYE, true, DEFAULT_VOLUME_CAP);
        assert_eq!(even.len(), 2);
    }

    #[test]
    fn a_loop_fades_when_the_volume_changes_and_restarts_when_the_machines_move() {
        let mut scapes = Scapes::default();
        scapes.report(true, CAMERA, Group::Kinetic, [3, 64, 0], 0.85);
        scapes.tick(1, EYE, true, DEFAULT_VOLUME_CAP);
        // The player walks away from it: at the next update the volume fades.
        scapes.report(true, CAMERA, Group::Kinetic, [3, 64, 0], 0.85);
        let walked = scapes.tick(5, [10.5, 64.0, 0.5], true, DEFAULT_VOLUME_CAP);
        assert!(walked.iter().any(|a| matches!(a, Action::FadeLoop { .. })), "{walked:?}");
        // A second machine far from the first moves the mean by more than a block.
        scapes.report(true, CAMERA, Group::Kinetic, [3, 64, 0], 0.85);
        scapes.report(true, CAMERA, Group::Kinetic, [9, 64, 0], 0.85);
        let moved = scapes.tick(10, EYE, true, DEFAULT_VOLUME_CAP);
        assert!(moved.iter().any(|a| matches!(a, Action::MoveLoop { .. })), "{moved:?}");
    }

    #[test]
    fn disabled_scapes_end_at_the_next_update() {
        let mut scapes = Scapes::default();
        scapes.report(true, CAMERA, Group::Cog, [3, 64, 0], 0.85);
        scapes.tick(1, EYE, true, DEFAULT_VOLUME_CAP);
        let off = scapes.tick(5, EYE, false, DEFAULT_VOLUME_CAP);
        assert!(off.iter().any(|a| matches!(a, Action::StopLoop { .. })));
        assert!(scapes.is_empty());
    }

    #[test]
    fn clear_stops_every_started_loop() {
        let mut scapes = Scapes::default();
        scapes.report(true, CAMERA, Group::Cog, [3, 64, 0], 0.85);
        scapes.tick(1, EYE, true, DEFAULT_VOLUME_CAP);
        scapes.report(true, CAMERA, Group::Kinetic, [3, 64, 0], 0.85); // never started
        let stops = scapes.clear();
        assert_eq!(stops.len(), 1);
        assert!(scapes.is_empty());
    }

    #[test]
    fn every_create_event_a_scape_names_is_an_entry() {
        assert!(names_known_events());
    }
}
