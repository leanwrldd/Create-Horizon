//! The Steam Whistle (`WhistleBlock`, `WhistleBlockEntity`, `WhistleExtenderBlock`,
//! `WhistleRenderer`, `WhistleSoundInstance`), ported from Create (MIT, the Creators of Create)
//! and checked against `mc1.21.1/dev` at 4148fd8. No game types.
//!
//! A whistle sits on a tank (on top, or on a side wall) and sounds while redstone powers it and
//! its boiler is heated. Taller whistles are built by using a Steam Whistle item on one: each
//! extension block lowers the note, by one semitone for a single, two for a double.

use crate::Dir;

/// `WhistleBlock.WhistleSize`: the octave. Its `ordinal` and serialised name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

impl Size {
    pub const ALL: [Size; 3] = [Size::Small, Size::Medium, Size::Large];

    pub const fn name(self) -> &'static str {
        match self {
            Size::Small => "small",
            Size::Medium => "medium",
            Size::Large => "large",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    pub const fn ordinal(self) -> i32 {
        match self {
            Size::Small => 0,
            Size::Medium => 1,
            Size::Large => 2,
        }
    }

    /// `BlockState.cycle(SIZE)`, what the wrench does (`WhistleBlock.getRotatedBlockState`).
    pub const fn next(self) -> Size {
        match self {
            Size::Small => Size::Medium,
            Size::Medium => Size::Large,
            Size::Large => Size::Small,
        }
    }

    /// `WhistleSoundInstance`: the sample for the octave (`AllSoundEvents.WHISTLE_HIGH`,
    /// `WHISTLE_MEDIUM`, `WHISTLE_LOW`).
    pub const fn sound(self) -> &'static str {
        match self {
            Size::Small => "create:whistle_high",
            Size::Medium => "create:whistle",
            Size::Large => "create:whistle_low",
        }
    }
}

/// `WhistleExtenderBlock.WhistleExtenderShape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Single,
    Double,
    DoubleConnected,
}

impl Shape {
    pub const ALL: [Shape; 3] = [Shape::Single, Shape::Double, Shape::DoubleConnected];

    pub const fn name(self) -> &'static str {
        match self {
            Shape::Single => "single",
            Shape::Double => "double",
            Shape::DoubleConnected => "double_connected",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// What stands above a whistle, one block at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Above {
    /// A Steam Whistle Extension of this shape.
    Extension(Shape),
    /// A block the next extension may replace (`BlockState.canBeReplaced`).
    Free,
    /// Anything else.
    Blocked,
}

/// `WhistleBlock.getAttachedDirection`: where the tank is.
pub fn attached_direction(wall: bool, facing: Dir) -> Dir {
    if wall { facing } else { Dir::Down }
}

/// `WhistleBlock.getStateForPlacement`: the `facing` and `wall` for a click on `face` by a player
/// facing `horizontal`. Clicking the top or bottom of a block puts the whistle on the floor, facing
/// as the player does; clicking a side puts it on the wall, facing the tank.
pub fn placement(face: Dir, horizontal: Dir) -> (Dir, bool) {
    if face.axis() == crate::Axis::Y {
        (horizontal, false)
    } else {
        (face.opposite(), true)
    }
}

/// `WhistleBlockEntity.updatePitch`: the pitch (semitones below the note of the whistle's size) from
/// the blocks above it. Each double (joined or not) adds 2, a single adds 1 and ends the column,
/// and nothing goes past 24 + 1.
pub fn pitch(mut above: impl FnMut(i32) -> Above) -> i32 {
    let mut new_pitch = 0;
    let mut height = 0;
    while new_pitch <= 24 {
        match above(height) {
            Above::Extension(Shape::Single) => {
                new_pitch += 1;
                break;
            }
            Above::Extension(_) => {}
            _ => break,
        }
        height += 1;
        new_pitch += 2;
    }
    new_pitch
}

/// `WhistleBlockEntity.getPitchId`.
pub fn pitch_id(pitch: i32, size: Size) -> i32 {
    pitch + 100 * size.ordinal()
}

/// `WhistleBlockEntity.tickAudio`: the playback rate, `2^(-pitch / 12)`.
pub fn pitch_factor(pitch: i32) -> f32 {
    2.0f64.powf(-f64::from(pitch) / 12.0) as f32
}

/// What using a Steam Whistle item on a whistle does (`WhistleBlock.incrementSize`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grow {
    /// The single extension `i` blocks above (1 is the first) becomes a double.
    Double(i32),
    /// A new single extension goes `i` blocks above.
    Place(i32),
    /// No room within six blocks.
    Nothing,
}

/// `WhistleBlock.incrementSize`: `above(i)` is the block `i` above the whistle (1 first), asked up
/// to six times.
pub fn grow(mut above: impl FnMut(i32) -> Above) -> Grow {
    for i in 1..=6 {
        match above(i) {
            Above::Extension(Shape::Single) => return Grow::Double(i),
            Above::Extension(_) => continue,
            Above::Free => return Grow::Place(i),
            Above::Blocked => return Grow::Nothing,
        }
    }
    Grow::Nothing
}

/// The pitch of the xylophone and the metal hit played as an extension grows
/// (`Math.pow(2, -(i * 2) / 12.0)` for a double, `-(i * 2 - 1)` for a new single).
pub fn grow_pitch(grow: Grow) -> Option<f32> {
    match grow {
        Grow::Double(i) => Some(2.0f64.powf(-f64::from(i * 2) / 12.0) as f32),
        Grow::Place(i) => Some(2.0f64.powf(-f64::from(i * 2 - 1) / 12.0) as f32),
        Grow::Nothing => None,
    }
}

/// `WhistleExtenderBlock.canSurvive`: it stands on a double extension or on a whistle.
pub fn extension_survives(below: Below) -> bool {
    matches!(
        below,
        Below::Extension(Shape::Double | Shape::DoubleConnected) | Below::Whistle
    )
}

/// What is under an extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Below {
    Extension(Shape),
    Whistle,
    Other,
}

/// `WhistleExtenderBlock.updateShape` from above: a double joins the extension above it.
pub fn next_shape(shape: Shape, above_is_extension: bool) -> Shape {
    let connected = shape == Shape::DoubleConnected;
    if !connected && above_is_extension {
        Shape::DoubleConnected
    } else if connected && !above_is_extension {
        Shape::Double
    } else {
        shape
    }
}

// --- the mouth -------------------------------------------------------------------------------------

/// `LerpedFloat.chase(target, .5f, Chaser.EXP)` and `(.., .4f, Chaser.LINEAR)` in closed form:
/// the animation `elapsed` ticks after it was `from` and started going to on (1) or off (0). On
/// halves the way left each tick; off falls 0.4 a tick.
pub fn animation(on: bool, from: f32, elapsed: f32) -> f32 {
    let elapsed = elapsed.max(0.0);
    if on {
        1.0 - (1.0 - from) * 0.5f32.powf(elapsed)
    } else {
        (from - 0.4 * elapsed).max(0.0)
    }
}

/// `WhistleRenderer`: how far the mouth rises in blocks, given the animation's value and whether it
/// is heading on, at `time` ticks (with the partial tick), wiggling while it sounds.
pub fn mouth_offset(value: f32, on: bool, time: f32, size: Size) -> f32 {
    let mut offset = value;
    if on && value > 0.5 {
        let progress = time / 8.0;
        offset -= (progress * (2.0 * std::f32::consts::PI) * (4 - size.ordinal()) as f32).sin() / 16.0;
    }
    offset * 4.0 / 16.0
}

/// `AllPartialModels.WHISTLE_MOUTH_*`.
pub const fn mouth_model(size: Size) -> &'static str {
    match size {
        Size::Large => "create:block/steam_whistle/large_mouth",
        Size::Medium => "create:block/steam_whistle/medium_mouth",
        Size::Small => "create:block/steam_whistle/small_mouth",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(blocks: &'static [Above]) -> impl FnMut(i32) -> Above {
        move |i| blocks.get(i as usize).copied().unwrap_or(Above::Blocked)
    }

    // `above(0)` is the first block above for `pitch`; `grow` counts from 1.
    fn grow_column(blocks: &'static [Above]) -> impl FnMut(i32) -> Above {
        move |i| blocks.get(i as usize - 1).copied().unwrap_or(Above::Blocked)
    }

    const SINGLE: Above = Above::Extension(Shape::Single);
    const DOUBLE: Above = Above::Extension(Shape::Double);
    const CONNECTED: Above = Above::Extension(Shape::DoubleConnected);

    #[test]
    fn sizes_cycle_and_name_their_sample() {
        assert_eq!(Size::Small.next(), Size::Medium);
        assert_eq!(Size::Medium.next(), Size::Large);
        assert_eq!(Size::Large.next(), Size::Small);
        assert_eq!(Size::from_name("large"), Some(Size::Large));
        assert_eq!(Size::from_name("huge"), None);
        assert_eq!(Size::Small.sound(), "create:whistle_high");
        assert_eq!(Size::Medium.sound(), "create:whistle");
        assert_eq!(Size::Large.sound(), "create:whistle_low");
        assert_eq!(Shape::from_name("double_connected"), Some(Shape::DoubleConnected));
    }

    #[test]
    fn a_whistle_stands_on_the_floor_or_the_wall() {
        // On top of a tank: the player's direction.
        assert_eq!(placement(Dir::Up, Dir::East), (Dir::East, false));
        assert_eq!(attached_direction(false, Dir::East), Dir::Down);
        // On the east face of a tank: facing the tank, west.
        assert_eq!(placement(Dir::East, Dir::East), (Dir::West, true));
        assert_eq!(attached_direction(true, Dir::West), Dir::West);
        // Under a tank, clicking the bottom face, still the floor case.
        assert_eq!(placement(Dir::Down, Dir::North), (Dir::North, false));
    }

    #[test]
    fn the_pitch_counts_semitones_down_the_column() {
        assert_eq!(pitch(column(&[Above::Blocked])), 0);
        assert_eq!(pitch(column(&[SINGLE])), 1);
        assert_eq!(pitch(column(&[DOUBLE])), 2);
        assert_eq!(pitch(column(&[CONNECTED, SINGLE])), 3);
        assert_eq!(pitch(column(&[CONNECTED, CONNECTED, DOUBLE])), 6);
        assert_eq!(pitch(column(&[DOUBLE, SINGLE])), 3);
        // A column of doubles stops at 26, past which nothing is read.
        static TALL: [Above; 20] = [CONNECTED; 20];
        assert_eq!(pitch(column(&TALL)), 26);
        assert_eq!(pitch_id(5, Size::Large), 205);
        assert_eq!(pitch_id(0, Size::Small), 0);
    }

    #[test]
    fn the_note_halves_in_rate_each_octave() {
        assert_eq!(pitch_factor(0), 1.0);
        assert!((pitch_factor(12) - 0.5).abs() < 1e-6);
        assert!((pitch_factor(24) - 0.25).abs() < 1e-6);
        assert!(pitch_factor(1) < 1.0 && pitch_factor(1) > pitch_factor(2));
    }

    #[test]
    fn using_a_whistle_on_one_builds_up_the_column() {
        assert_eq!(grow(grow_column(&[Above::Free])), Grow::Place(1));
        assert_eq!(grow(grow_column(&[Above::Blocked])), Grow::Nothing);
        assert_eq!(grow(grow_column(&[SINGLE])), Grow::Double(1));
        assert_eq!(grow(grow_column(&[DOUBLE, Above::Free])), Grow::Place(2));
        assert_eq!(grow(grow_column(&[CONNECTED, SINGLE])), Grow::Double(2));
        assert_eq!(grow(grow_column(&[DOUBLE, DOUBLE, DOUBLE, Above::Free])), Grow::Place(4));
        // Six doubles fill the search.
        static FULL: [Above; 7] = [CONNECTED, CONNECTED, CONNECTED, CONNECTED, CONNECTED, CONNECTED, Above::Free];
        assert_eq!(grow(grow_column(&FULL)), Grow::Nothing);
        assert_eq!(grow_pitch(Grow::Nothing), None);
        assert!((grow_pitch(Grow::Place(1)).unwrap() - 2.0f32.powf(-1.0 / 12.0)).abs() < 1e-6);
        assert!((grow_pitch(Grow::Double(3)).unwrap() - 2.0f32.powf(-6.0 / 12.0)).abs() < 1e-6);
    }

    #[test]
    fn an_extension_needs_a_whistle_or_a_double_under_it() {
        assert!(extension_survives(Below::Whistle));
        assert!(extension_survives(Below::Extension(Shape::Double)));
        assert!(extension_survives(Below::Extension(Shape::DoubleConnected)));
        assert!(!extension_survives(Below::Extension(Shape::Single)));
        assert!(!extension_survives(Below::Other));
    }

    #[test]
    fn doubles_join_the_extension_above() {
        assert_eq!(next_shape(Shape::Double, true), Shape::DoubleConnected);
        assert_eq!(next_shape(Shape::DoubleConnected, false), Shape::Double);
        assert_eq!(next_shape(Shape::DoubleConnected, true), Shape::DoubleConnected);
        assert_eq!(next_shape(Shape::Double, false), Shape::Double);
        assert_eq!(next_shape(Shape::Single, false), Shape::Single);
    }

    #[test]
    fn the_mouth_rises_quickly_and_falls_slowly() {
        assert_eq!(animation(true, 0.0, 0.0), 0.0);
        assert_eq!(animation(true, 0.0, 1.0), 0.5);
        assert_eq!(animation(true, 0.0, 2.0), 0.75);
        assert!(animation(true, 0.0, 20.0) > 0.999);
        assert!((animation(false, 1.0, 1.0) - 0.6).abs() < 1e-6);
        assert_eq!(animation(false, 1.0, 5.0), 0.0);
        assert_eq!(animation(false, 0.2, 1.0), 0.0);
        // From part way: on from 0.5 goes to 0.75.
        assert_eq!(animation(true, 0.5, 1.0), 0.75);
    }

    #[test]
    fn the_mouth_wiggles_only_while_sounding_high() {
        // At rest: no offset.
        assert_eq!(mouth_offset(0.0, false, 12.0, Size::Medium), 0.0);
        // Rising but not past half: a plain rise.
        assert_eq!(mouth_offset(0.5, true, 3.0, Size::Medium), 0.5 * 4.0 / 16.0);
        // Sounding: a quarter block up less the wiggle (at most 1/16 of the unit), 4 - ordinal times
        // a turn each 8 ticks: at time 0 the sine is 0.
        assert_eq!(mouth_offset(1.0, true, 0.0, Size::Small), 4.0 / 16.0);
        let wiggled = mouth_offset(1.0, true, 1.0, Size::Large);
        assert!((wiggled - 4.0 / 16.0).abs() <= 4.0 / 16.0 / 16.0 + 1e-6);
        assert_ne!(wiggled, 4.0 / 16.0);
        // Falling: no wiggle however high.
        assert_eq!(mouth_offset(1.0, false, 1.0, Size::Large), 4.0 / 16.0);
    }
}
