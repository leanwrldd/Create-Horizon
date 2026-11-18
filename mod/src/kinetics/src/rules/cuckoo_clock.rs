//! The Cuckoo Clock's rules: the time its hands show, when its bird comes out, and the
//! sounds, door angles and the explosion of the animation. Ported from Create
//! `mc1.21.1/dev` at 4148fd8 (MIT, the Creators of Create): `CuckooClockBlockEntity.tick`,
//! `startAnimation`, `CuckooClockRenderer.renderSafe`, `AllBlocks.CUCKOO_CLOCK`
//! (`CStress.setImpact(1)`). No game types.

/// Stress units per RPM (`BuilderTransformers.cuckooClock`: `CStress.setImpact(1)`).
pub const IMPACT: f32 = 1.0;

/// `animationProgress` passing this ends the animation (`value > 100`).
pub const LAST_PROGRESS: f32 = 100.0;

/// `CuckooClockBlockEntity.Animation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Animation {
    Pig,
    Creeper,
    Surprise,
    None,
}

impl Animation {
    /// The names `NBTHelper.writeEnum` saves (`Animation`).
    pub fn name(self) -> &'static str {
        match self {
            Animation::Pig => "PIG",
            Animation::Creeper => "CREEPER",
            Animation::Surprise => "SURPRISE",
            Animation::None => "NONE",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Animation::Pig, Animation::Creeper, Animation::Surprise, Animation::None]
            .into_iter()
            .find(|a| a.name() == name)
    }
}

/// The time the clock shows: `(hours, minutes)` from the world's day time.
/// `dayTime = (int) ((level.getDayTime() * (isNatural ? 1 : 24)) % 24000)`,
/// `hours = (dayTime / 1000 + 6) % 24`, `minutes = (dayTime % 1000) * 60 / 1000`.
pub fn clock_time(day_time: i64, natural: bool) -> (i32, i32) {
    let day_time = (day_time.wrapping_mul(if natural { 1 } else { 24 }) % 24000) as i32;
    ((day_time / 1000 + 6) % 24, (day_time % 1000) * 60 / 1000)
}

/// The bird that comes out at this time, while none is out (`tick`): the pig at noon
/// (`hours == 12 && minutes < 5`) and the creeper at 18:32 to 18:35.
pub fn due(hours: i32, minutes: i32) -> Option<Animation> {
    if hours == 12 && minutes < 5 {
        return Some(Animation::Pig);
    }
    if hours == 18 && minutes < 36 && minutes > 31 {
        return Some(Animation::Creeper);
    }
    None
}

/// `startAnimation`: a Mysterious Cuckoo Clock (`CuckooClockBlock.containsSurprise`) always
/// brings the surprise out.
pub fn started(animation: Animation, mysterious: bool) -> Animation {
    if animation != Animation::None && mysterious {
        Animation::Surprise
    } else {
        animation
    }
}

/// The server's step (`value` is the progress before this tick's increment): whether the
/// animation is over (`value > 100`) and whether the Mysterious Clock goes off now
/// (`Mth.equal(animationProgress.getValue(), 50)` after the increment).
pub fn explodes(animation: Animation, value: f32) -> bool {
    animation == Animation::Surprise && (value + 1.0 - 50.0).abs() < 1e-5
}

/// A sound for the client's tick at progress `value` (`CuckooClockBlockEntity.tick`, client
/// half): `(sound event, volume, pitch)`.
pub type Cue = (&'static str, f32, f32);

/// The sounds at progress `value` of an animation (the value before this tick's increment).
pub fn cues(animation: Animation, value: i32) -> Vec<Cue> {
    let surprise = animation == Animation::Surprise;
    let mut out = Vec::new();
    if value == 1 {
        out.push(("minecraft:block.note_block.chime", 2.0, 0.5));
    }
    if value == 21 {
        out.push(("minecraft:block.note_block.chime", 2.0, 0.793_701));
    }
    if value == 40 && surprise {
        out.push(("minecraft:entity.tnt.primed", 1.0, 1.0));
    }
    let step = if surprise { 3 } else { 15 };
    let mut phase = 30;
    while phase <= 60 {
        if value == phase - step / 3 {
            out.push(("minecraft:block.chest.open", 1.0 / 16.0, 2.0));
        }
        if value == phase {
            out.push(if animation == Animation::Pig {
                ("minecraft:entity.pig.ambient", 0.25, 1.0)
            } else {
                ("minecraft:entity.creeper.hurt", 0.25, 3.0)
            });
        }
        if value == phase + step / 3 {
            out.push(("minecraft:block.chest.close", 1.0 / 16.0, 2.0));
        }
        phase += step;
    }
    out
}

/// Whether the surprise smokes at this progress (`value > 30 && isSurprise`).
pub fn smokes(animation: Animation, value: i32) -> bool {
    animation == Animation::Surprise && value > 30
}

/// The tick of a clock with no animation running (`AnimationTickHolder.getTicks()`): a hat at
/// pitch 2 every 32 ticks and at 1.5 every other 16.
pub fn idle_tick(ticks: i64, natural: bool) -> Option<Cue> {
    let hat = "minecraft:block.note_block.hat";
    if natural {
        if ticks % 32 == 0 {
            return Some((hat, 1.0 / 16.0, 2.0));
        }
        if ticks % 16 == 0 {
            return Some((hat, 1.0 / 16.0, 1.5));
        }
    } else {
        // A world with no natural time (`!isNatural`): the clock ticks fast, shows 24 times the
        // time and never brings a bird out.
        if ticks % 6 == 0 {
            return Some((hat, 1.0 / 16.0, 2.0));
        }
        if ticks % 3 == 0 {
            return Some((hat, 1.0 / 16.0, 1.5));
        }
    }
    None
}

/// `moveHands`' targets: `360 / 12 * (hours % 12)` and `360 / 60 * minutes`, in degrees.
pub fn hand_angles(hours: i32, minutes: i32) -> (f32, f32) {
    ((360 / 12 * (hours % 12)) as f32, (360 / 60 * minutes) as f32)
}

/// The door angle at animation progress `value` (`CuckooClockRenderer.renderSafe`): open to
/// 135 degrees at each phase and closed between.
pub fn door_angle(animation: Animation, value: f32) -> f32 {
    let step = if animation == Animation::Surprise { 3 } else { 15 };
    let third = (step / 3) as f32;
    let mut angle = 0.0;
    let mut phase = 30;
    while phase <= 60 {
        let local = value - phase as f32;
        // `if (local < -step / 3) continue;` leaves `angle` as the last phase made it.
        if local >= -third {
            if local < 0.0 {
                angle = lerp((value - (phase as f32 - 5.0)) / 5.0, 0.0, 135.0);
            } else if local < third {
                angle = 135.0;
            } else if local < (2 * step / 3) as f32 {
                angle = lerp((value - (phase as f32 + 5.0)) / 5.0, 135.0, 0.0);
            }
        }
        phase += step;
    }
    angle
}

/// `Mth.lerp(delta, start, end)`.
fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * (end - start)
}

/// How far the figure is pushed out of the clock for a door angle (`offset`): `10 / 16` shut
/// (hidden), `10 / 16 - 1 / 2` fully open.
pub fn figure_offset(door_angle: f32) -> f32 {
    -(door_angle / 135.0) / 2.0 + 10.0 / 16.0
}

/// `AngleHelper.horizontalAngle(facing.getCounterClockWise())` for a horizontal `facing`'s
/// name: how the hands, doors and figure (modelled facing west) are turned.
pub fn model_angle(facing: &str) -> f32 {
    // getCounterClockWise: north -> west, west -> south, south -> east, east -> north.
    match facing {
        "north" => 270.0,
        "west" => 0.0,
        "south" => 90.0,
        "east" => 180.0,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overworld_day_starts_at_six() {
        assert_eq!(clock_time(0, true), (6, 0));
        assert_eq!(clock_time(6000, true), (12, 0));
        assert_eq!(clock_time(12500, true), (18, 30));
        assert_eq!(clock_time(18000, true), (0, 0));
        // 1000 ticks an hour, 60 minutes: 500 ticks is the half hour.
        assert_eq!(clock_time(6500, true), (12, 30));
        assert_eq!(clock_time(24000, true), (6, 0));
    }

    #[test]
    fn a_world_without_natural_time_runs_the_clock_24_times_as_fast() {
        // 1000 ticks of day time is one hour, so 1000 / 24 ticks a clock hour.
        assert_eq!(clock_time(0, false), (6, 0));
        assert_eq!(clock_time(1, false), (6, 1));
        assert_eq!(clock_time(500, false), (18, 0));
    }

    #[test]
    fn the_pig_comes_out_at_noon_and_the_creeper_at_half_past_six() {
        assert_eq!(due(12, 0), Some(Animation::Pig));
        assert_eq!(due(12, 4), Some(Animation::Pig));
        assert_eq!(due(12, 5), None);
        assert_eq!(due(18, 31), None);
        assert_eq!(due(18, 32), Some(Animation::Creeper));
        assert_eq!(due(18, 35), Some(Animation::Creeper));
        assert_eq!(due(18, 36), None);
        assert_eq!(due(3, 0), None);
    }

    #[test]
    fn the_mysterious_clock_always_brings_the_surprise() {
        assert_eq!(started(Animation::Pig, true), Animation::Surprise);
        assert_eq!(started(Animation::Creeper, true), Animation::Surprise);
        assert_eq!(started(Animation::Pig, false), Animation::Pig);
        assert_eq!(started(Animation::None, true), Animation::None);
    }

    #[test]
    fn the_surprise_goes_off_on_the_fiftieth_step() {
        assert!(!explodes(Animation::Surprise, 48.0));
        assert!(explodes(Animation::Surprise, 49.0));
        assert!(!explodes(Animation::Surprise, 50.0));
        assert!(!explodes(Animation::Pig, 49.0));
    }

    #[test]
    fn the_cues_follow_the_timeline() {
        assert_eq!(cues(Animation::Pig, 1), vec![("minecraft:block.note_block.chime", 2.0, 0.5)]);
        assert_eq!(cues(Animation::Pig, 21)[0].2, 0.793_701);
        // The pig's door: opens 5 ticks before 30, oinks at 30, closes at 35.
        assert_eq!(cues(Animation::Pig, 25)[0].0, "minecraft:block.chest.open");
        assert_eq!(cues(Animation::Pig, 30)[0].0, "minecraft:entity.pig.ambient");
        assert_eq!(cues(Animation::Pig, 35)[0].0, "minecraft:block.chest.close");
        assert_eq!(cues(Animation::Pig, 45)[0].0, "minecraft:entity.pig.ambient");
        assert_eq!(cues(Animation::Pig, 60)[0].0, "minecraft:entity.pig.ambient");
        // The creeper hurts at pitch 3, and the surprise primes at 40 and flaps every 3 ticks.
        assert_eq!(cues(Animation::Creeper, 30)[0], ("minecraft:entity.creeper.hurt", 0.25, 3.0));
        assert_eq!(cues(Animation::Surprise, 40)[0].0, "minecraft:entity.tnt.primed");
        assert_eq!(cues(Animation::Surprise, 33)[0].0, "minecraft:entity.creeper.hurt");
        assert!(cues(Animation::Pig, 2).is_empty());
        assert!(smokes(Animation::Surprise, 31) && !smokes(Animation::Surprise, 30));
    }

    #[test]
    fn the_ticks_of_an_idle_clock() {
        assert_eq!(idle_tick(0, true).map(|c| c.2), Some(2.0));
        assert_eq!(idle_tick(16, true).map(|c| c.2), Some(1.5));
        assert_eq!(idle_tick(5, true), None);
        assert_eq!(idle_tick(6, false).map(|c| c.2), Some(2.0));
        assert_eq!(idle_tick(3, false).map(|c| c.2), Some(1.5));
    }

    #[test]
    fn the_hands_point_at_the_time() {
        assert_eq!(hand_angles(0, 0), (0.0, 0.0));
        assert_eq!(hand_angles(15, 30), (90.0, 180.0));
        assert_eq!(hand_angles(12, 59), (0.0, 354.0));
    }

    #[test]
    fn the_door_swings_open_at_each_phase() {
        assert_eq!(door_angle(Animation::Pig, 10.0), 0.0);
        assert_eq!(door_angle(Animation::Pig, 30.0), 135.0);
        assert!(door_angle(Animation::Pig, 27.5) > 0.0 && door_angle(Animation::Pig, 27.5) < 135.0);
        // Closing from 35 to 40 (108 degrees at 36), shut at 40, open again at 45.
        assert!((door_angle(Animation::Pig, 36.0) - 108.0).abs() < 1e-3);
        assert_eq!(door_angle(Animation::Pig, 40.0), 0.0);
        assert_eq!(door_angle(Animation::Pig, 45.0), 135.0);
        assert_eq!(figure_offset(0.0), 10.0 / 16.0);
        assert_eq!(figure_offset(135.0), 10.0 / 16.0 - 0.5);
    }

    #[test]
    fn the_model_turns_so_its_face_follows_the_block() {
        assert_eq!(model_angle("north"), 270.0);
        assert_eq!(model_angle("east"), 180.0);
        assert_eq!(model_angle("south"), 90.0);
        assert_eq!(model_angle("west"), 0.0);
    }

    #[test]
    fn animations_round_trip_their_saved_names() {
        for a in [Animation::Pig, Animation::Creeper, Animation::Surprise, Animation::None] {
            assert_eq!(Animation::from_name(a.name()), Some(a));
        }
    }
}
