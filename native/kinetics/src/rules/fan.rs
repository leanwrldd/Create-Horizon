//! The Encased Fan: its air current, what stands in it, and how items in it are processed.
//! Ported from Create's `EncasedFanBlock`, `EncasedFanBlockEntity`, `AirCurrent`,
//! `IAirCurrentSource`, `FanProcessing`, `AllFanProcessingTypes` and `CKinetics` (MIT, the
//! Creators of Create), checked against Create `mc1.21.1/dev` at 4148fd8.
//!
//! No game types here: the glue (`native/api/src/machines/fan.rs`) reads the world and hands it
//! in. On the rotation network a fan is one shaft out of its back (`EncasedFanBlock.
//! hasShaftTowards`: `face == facing.getOpposite()`) that costs [`IMPACT`] (`AllBlocks.ENCASED_FAN`:
//! `CStress.setImpact(2.0)`).
//!
//! Known differences from Create (for the README):
//! - the air current is rebuilt every [`BLOCK_CHECK_RATE`] ticks and when the speed changes, not
//!   also when a neighbour changes (Horizon's neighbour hook can't reach a block entity);
//! - copycat blocks are not looked through (there are none yet);
//! - `fallDistance` isn't reset for entities in the current (the Host can't set it);
//! - particles and the wind sound are left out.

use crate::{Axis, Dir};

/// Stress units per RPM (`AllBlocks.ENCASED_FAN`: `CStress.setImpact(2.0)`).
pub const IMPACT: f32 = 2.0;
/// `CKinetics.fanPushDistance`.
pub const PUSH_DISTANCE: f32 = 20.0;
/// `CKinetics.fanPullDistance`.
pub const PULL_DISTANCE: f32 = 20.0;
/// `CKinetics.fanBlockCheckRate`: ticks between looks for anything blocking the air flow.
pub const BLOCK_CHECK_RATE: i32 = 30;
/// `CKinetics.fanRotationArgmax`: the speed at which the longest reach is made.
pub const ROTATION_ARGMAX: f32 = 256.0;
/// `CKinetics.fanProcessingTime`: ticks a stack of up to 16 takes to process.
pub const PROCESSING_TIME: i32 = 150;
/// `EncasedFanBlockEntity.tick`: ticks between searches for entities in the current.
pub const ENTITY_SEARCH: i32 = 5;
/// `AirCurrent.tickAffectedEntities`: `maxAcceleration`.
pub const MAX_ACCELERATION: f32 = 5.0;

/// `Mth.lerp`.
pub fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * (end - start)
}

/// `IAirCurrentSource.getMaxDistance`: how far the current reaches at `speed` RPM (signed).
pub fn max_distance(speed: f32) -> f32 {
    let factor = (speed.abs() / ROTATION_ARGMAX).min(1.0);
    let push = lerp(factor, 3.0, PUSH_DISTANCE);
    let pull = lerp(factor, 3.0, PULL_DISTANCE);
    if speed > 0.0 { push } else { pull }
}

/// `EncasedFanBlockEntity.getAirFlowDirection`: the way the air moves, `None` standing still.
/// `convertToDirection` turns the speed's sign around for a facing on the negative end of its
/// axis.
pub fn flow_direction(speed: f32, facing: Dir) -> Option<Dir> {
    if speed == 0.0 {
        return None;
    }
    let converted = if facing.sign() > 0 { speed } else { -speed };
    Some(if converted > 0.0 { facing } else { facing.opposite() })
}

/// `AirCurrent.pushing`: the air leaves the front of the fan.
pub fn pushing(speed: f32, facing: Dir) -> bool {
    flow_direction(speed, facing) == Some(facing)
}

// --- how far the air gets (`AirCurrent.getFlowLimit`) -----------------------------------------

/// A box `[min x, min y, min z, max x, max y, max z]` in blocks, relative to its block.
pub type Aabb = [f64; 6];

/// What the glue found `i + 1` blocks from the fan.
#[derive(Clone, Debug, PartialEq)]
pub enum Passage {
    /// `!world.isLoaded(pos)`.
    Unloaded,
    /// `shouldAlwaysPass`: a block of the `create:fan_transparent` tag.
    Transparent,
    /// The block's collision boxes (empty for a shapeless block).
    Boxes(Vec<Aabb>),
}

/// `shape == Shapes.block()`: one box that fills the block.
fn is_full_block(boxes: &[Aabb]) -> bool {
    boxes.len() == 1
        && boxes[0]
            .iter()
            .enumerate()
            .all(|(i, v)| (v - if i < 3 { 0.0 } else { 1.0 }).abs() < 1e-3)
}

/// `AirCurrent.DEPTH_TEST_COORDINATES`.
const DEPTH_TEST_COORDINATES: [[f64; 2]; 5] =
    [[0.25, 0.25], [0.25, 0.75], [0.5, 0.5], [0.75, 0.25], [0.75, 0.75]];

/// `AirCurrent.findMaxDepth`: how deep a shape reaches into the block along `direction`, tested
/// at five points across it; infinite when one of them goes through a hole. `VoxelShape.min`
/// and `max` read the cell a point is in, a cell being `[low, high)`.
pub fn shape_depth(boxes: &[Aabb], direction: Dir) -> f64 {
    let axis = direction.axis().index();
    // `AxisCycle`: the point's two coordinates belong to the axes after `axis` in the
    // cycle x, y, z (the five points are symmetric, so their order doesn't matter).
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let positive = direction.sign() > 0;
    let mut deepest = 0.0_f64;
    for [ca, cb] in DEPTH_TEST_COORDINATES {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for shape in boxes {
            if shape[a] <= ca && ca < shape[a + 3] && shape[b] <= cb && cb < shape[b + 3] {
                low = low.min(shape[axis]);
                high = high.max(shape[axis + 3]);
            }
        }
        let depth = if positive {
            if low == f64::INFINITY {
                return f64::INFINITY;
            }
            low
        } else {
            if high == f64::NEG_INFINITY {
                return f64::INFINITY;
            }
            1.0 - high
        };
        deepest = deepest.max(depth);
    }
    deepest
}

/// `AirCurrent.getFlowLimit`: how far the air gets from a fan whose reach is `max`, looking at
/// the blocks `1, 2, ...` away along `facing`; `probe(i)` describes the block `i + 1` away.
pub fn flow_limit(max: f32, facing: Dir, mut probe: impl FnMut(i32) -> Passage) -> f32 {
    let mut i = 0;
    while (i as f32) < max {
        match probe(i) {
            Passage::Unloaded => return i as f32,
            Passage::Transparent => {}
            Passage::Boxes(boxes) => {
                if boxes.is_empty() {
                    i += 1;
                    continue;
                }
                if is_full_block(&boxes) {
                    return i as f32;
                }
                let depth = shape_depth(&boxes, facing);
                if depth != f64::INFINITY {
                    return ((f64::from(i) + depth + 1.0 / 32.0) as f32).min(max);
                }
            }
        }
        i += 1;
    }
    max
}

/// `AirCurrent.getLimit`: the number of blocks the current reaches into, partly or whole.
pub fn limit(max_distance: f32) -> i32 {
    if (max_distance as i32) as f32 == max_distance {
        max_distance as i32
    } else {
        max_distance as i32 + 1
    }
}

/// `AirCurrent.bounds`: the box entities must touch to be caught, from the block in front of
/// the fan `reach` (the limited distance) blocks on; `None` under a quarter of a block.
pub fn bounds(fan: [i32; 3], direction: Dir, reach: f32) -> Option<([f64; 3], [f64; 3])> {
    if reach < 0.25 {
        return None;
    }
    let mut min = fan.map(f64::from);
    let mut max = min.map(|v| v + 1.0);
    let axis = direction.axis().index();
    // `new AABB(front).expandTowards(scale)` or `.contract(scale).move(scale)`: either way the
    // box is `reach` long from the fan's face.
    if direction.sign() > 0 {
        min[axis] = f64::from(fan[axis] + 1);
        max[axis] = f64::from(fan[axis] + 1) + f64::from(reach);
    } else {
        max[axis] = f64::from(fan[axis]);
        min[axis] = f64::from(fan[axis]) - f64::from(reach);
    }
    Some((min, max))
}

// --- what is in the air (`FanProcessingType`) -------------------------------------------------

/// `AllFanProcessingTypes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Splashing,
    Haunting,
    Smoking,
    Blasting,
}

impl Kind {
    /// Highest priority first: the order `FanProcessingType.getAt` asks them in
    /// (`getPriority`: splashing 400, haunting 300, smoking 200, blasting 100).
    pub const BY_PRIORITY: [Kind; 4] = [Kind::Splashing, Kind::Haunting, Kind::Smoking, Kind::Blasting];

    /// The id Create registers it under (`Create.asResource(name)`).
    pub const fn id(self) -> &'static str {
        match self {
            Kind::Splashing => "create:splashing",
            Kind::Haunting => "create:haunting",
            Kind::Smoking => "create:smoking",
            Kind::Blasting => "create:blasting",
        }
    }

    /// `AllFanProcessingTypes.parseLegacy`: an id, or Create's old upper-case names.
    pub fn parse(text: &str) -> Option<Kind> {
        Kind::BY_PRIORITY
            .into_iter()
            .find(|k| {
                k.id() == text
                    || (k.id()[7..].eq_ignore_ascii_case(text)
                        && text.chars().all(|c| c.is_ascii_uppercase()))
            })
    }

    /// The block tag of this type's catalysts (`AllBlockTags.FAN_PROCESSING_CATALYSTS_*`).
    pub const fn catalyst_tag(self) -> &'static str {
        match self {
            Kind::Splashing => "create:fan_processing_catalysts/splashing",
            Kind::Haunting => "create:fan_processing_catalysts/haunting",
            Kind::Smoking => "create:fan_processing_catalysts/smoking",
            Kind::Blasting => "create:fan_processing_catalysts/blasting",
        }
    }

    /// The recipe type a type reads, `None` for those that use vanilla's cooking recipes.
    pub const fn recipe_type(self) -> Option<&'static str> {
        match self {
            Kind::Splashing => Some("create:splashing"),
            Kind::Haunting => Some("create:haunting"),
            Kind::Smoking | Kind::Blasting => None,
        }
    }
}

/// `BlazeBurnerBlock.HeatLevel`, the `blaze` property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Heat {
    None,
    Smouldering,
    Fading,
    Kindled,
    Seething,
}

impl Heat {
    pub fn from_name(name: &str) -> Option<Heat> {
        Some(match name {
            "none" => Heat::None,
            "smouldering" => Heat::Smouldering,
            "fading" => Heat::Fading,
            "kindled" => Heat::Kindled,
            "seething" => Heat::Seething,
            _ => return None,
        })
    }
}

/// `LitBlazeBurnerBlock.FlameType`, the `flame_type` property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flame {
    Regular,
    Soul,
}

impl Flame {
    pub fn from_name(name: &str) -> Option<Flame> {
        match name {
            "regular" => Some(Flame::Regular),
            "soul" => Some(Flame::Soul),
            _ => None,
        }
    }
}

/// What the glue read at one position of the current.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    /// `FAN_PROCESSING_CATALYSTS_SPLASHING` fluid tag: water, flowing or not.
    pub water: bool,
    /// `FAN_PROCESSING_CATALYSTS_BLASTING` fluid tag: lava.
    pub lava: bool,
    /// The block is in the block tag of each type's catalysts.
    pub in_splashing: bool,
    pub in_haunting: bool,
    pub in_smoking: bool,
    pub in_blasting: bool,
    /// The block is in `minecraft:campfires`.
    pub campfire: bool,
    /// Its `lit` property, if it has one.
    pub lit: Option<bool>,
    /// Its `blaze` property (`BlazeBurnerBlock.HEAT_LEVEL`), if it has one.
    pub heat: Option<Heat>,
    /// Its `flame_type` property (`LitBlazeBurnerBlock.FLAME_TYPE`), if it has one.
    pub flame: Option<Flame>,
}

impl Cell {
    fn unlit_campfire(&self) -> bool {
        self.campfire && self.lit == Some(false)
    }

    /// `FanProcessingType.isValidAt`, one type's own rule.
    pub fn is_valid_for(&self, kind: Kind) -> bool {
        match kind {
            // `BlastingType`.
            Kind::Blasting => {
                if self.lava {
                    return true;
                }
                if self.in_blasting {
                    return self.heat.is_none_or(|heat| heat >= Heat::Fading);
                }
                false
            }
            // `HauntingType`.
            Kind::Haunting => {
                if !self.in_haunting {
                    return false;
                }
                if self.unlit_campfire() {
                    return false;
                }
                self.flame.is_none_or(|flame| flame == Flame::Soul)
            }
            // `SmokingType`.
            Kind::Smoking => {
                if !self.in_smoking {
                    return false;
                }
                if self.unlit_campfire() {
                    return false;
                }
                if self.flame.is_some_and(|flame| flame != Flame::Regular) {
                    return false;
                }
                self.heat.is_none_or(|heat| heat == Heat::Smouldering)
            }
            // `SplashingType`.
            Kind::Splashing => self.water || self.in_splashing,
        }
    }

    /// `FanProcessingType.getAt`: the first type, by priority, that holds here.
    pub fn kind(&self) -> Option<Kind> {
        Kind::BY_PRIORITY.into_iter().find(|kind| self.is_valid_for(*kind))
    }
}

/// The fluid a block holds, for the blocks whose fluid state isn't their own name: Create asks
/// `level.getFluidState(pos)`. `waterlogged` is the block's property of that name.
pub fn holds_water(block: &str, waterlogged: bool) -> bool {
    waterlogged
        || matches!(
            block,
            "minecraft:water"
                | "minecraft:bubble_column"
                | "minecraft:kelp"
                | "minecraft:kelp_plant"
                | "minecraft:seagrass"
                | "minecraft:tall_seagrass"
        )
}

/// A stretch of the current with one type (`AirCurrent.AirCurrentSegment`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub kind: Option<Kind>,
    pub start: i32,
    pub end: i32,
}

/// `AirCurrent.rebuild`'s walk: the segments of a current reaching `max_distance` (already
/// limited), given the type at each block `1..=limit` away (`kind_at(i)`). The type carries on
/// until another is found, in the order the air travels.
pub fn segments(
    max_distance: f32,
    pushing: bool,
    mut kind_at: impl FnMut(i32) -> Option<Kind>,
) -> Vec<Segment> {
    let limit = limit(max_distance);
    let (search_start, search_end, step) = if pushing { (1, limit, 1) } else { (limit, 1, -1) };
    let to_offset = if pushing { -1 } else { 0 };
    let mut out = Vec::new();
    let mut kind = None;
    let mut current: Option<Segment> = None;
    let mut i = search_start;
    while i * step <= search_end * step {
        if let Some(found) = kind_at(i) {
            kind = Some(found);
        }
        match current {
            None => {
                current = Some(Segment { kind, start: i + to_offset, end: 0 });
            }
            Some(mut segment) if segment.kind != kind => {
                segment.end = i + to_offset;
                out.push(segment);
                current = Some(Segment { kind, start: i + to_offset, end: 0 });
            }
            Some(_) => {}
        }
        i += step;
    }
    if let Some(mut segment) = current {
        segment.end = search_end + step + to_offset;
        out.push(segment);
    }
    out
}

/// `AirCurrent.getTypeAt`: the type `offset` blocks from the fan's face.
pub fn kind_at_offset(
    segments: &[Segment],
    max_distance: f32,
    pushing: bool,
    offset: f32,
) -> Option<Kind> {
    if !(offset >= 0.0 && offset <= max_distance) {
        return None;
    }
    if pushing {
        segments.iter().find(|s| offset <= s.end as f32).and_then(|s| s.kind)
    } else {
        segments.iter().find(|s| offset >= s.end as f32).and_then(|s| s.kind)
    }
}

/// `VecHelper.alignedDistanceToFace`: how far `position` is, along the fan's axis, from the
/// plane of the face the air leaves by.
pub fn aligned_distance_to_face(fan: [i32; 3], direction: Dir, position: [f64; 3]) -> f64 {
    let axis = direction.axis().index();
    let face = fan[axis] + i32::from(direction.sign() > 0);
    f64::from(position[axis] as f32 - face as f32).abs()
}

// --- pushing entities (`AirCurrent.tickAffectedEntities`) --------------------------------------

/// An entity's velocity after one tick in the current: `flow` is the way the air moves,
/// `distance_to_fan` the distance from the entity to the middle of the fan block (Create's
/// `entityDistanceOld`), `max_distance` the current's limited reach.
pub fn push_velocity(
    previous: [f64; 3],
    flow: Dir,
    speed: f32,
    sneaking: bool,
    distance_to_fan: f64,
    max_distance: f32,
) -> [f64; 3] {
    let speed = speed.abs();
    let sneak_modifier: f32 = if sneaking { 4096.0 } else { 512.0 };
    // Create divides by zero for an entity exactly at the middle of the fan; keep it finite.
    let distance = distance_to_fan.max(1e-4);
    let acceleration = (f64::from(speed / sneak_modifier) / (distance / f64::from(max_distance))) as f32;
    let max = f64::from(MAX_ACCELERATION);
    let normal = flow.normal();
    let mut velocity = previous;
    for axis in 0..3 {
        let wanted = f64::from(normal[axis] as f32 * acceleration);
        let delta = (wanted - previous[axis]).clamp(-max, max);
        velocity[axis] = previous[axis] + delta * f64::from(1.0_f32 / 8.0);
    }
    velocity
}

// --- processing items (`FanProcessing`) --------------------------------------------------------

/// `FanProcessing.decrementProcessingTime`'s time for a fresh stack: ticks until it is done.
pub fn processing_time(count: u32) -> i32 {
    let modifier = (count.max(1) as i32 - 1) / 16 + 1;
    PROCESSING_TIME * modifier + 1
}

/// What an item entity remembers of its processing: Create's `CreateData.Processing`
/// (`Type`, `Time`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// The type's id, as written; `None` while the item has not been in a current.
    pub kind: Option<String>,
    pub time: i32,
}

/// What `FanProcessing.canProcess(ItemEntity, type)` can say without asking the recipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Known {
    Yes,
    No,
    /// Ask the type (`type.canProcess(stack, level)`).
    Ask,
}

/// `FanProcessing.canProcess(ItemEntity, FanProcessingType)` up to the recipe lookup.
pub fn can_process(progress: Option<&Progress>, kind: Kind) -> Known {
    if let Some(progress) = progress {
        let same = progress.kind.as_deref().and_then(Kind::parse) == Some(kind);
        if !same {
            return Known::Ask;
        }
        if progress.time >= 0 {
            return Known::Yes;
        }
        if progress.time == -1 {
            return Known::No;
        }
    }
    Known::Ask
}

/// `FanProcessing.decrementProcessingTime`: one tick of processing for a stack of `count`;
/// answers the time left. Processing happens when it reaches 0.
pub fn decrement(progress: &mut Progress, kind: Kind, count: u32) -> i32 {
    if progress.kind.as_deref().and_then(Kind::parse) != Some(kind) {
        progress.kind = Some(kind.id().to_owned());
        progress.time = processing_time(count);
    }
    progress.time -= 1;
    progress.time
}

// --- what a current does to entities (`AllFanProcessingTypes.*.affectEntity`) ------------------

/// Ticks an entity burns for in lava-heated air (`igniteForSeconds(10)`).
pub const BLASTING_FIRE_TICKS: i32 = 200;
/// Damage in lava-heated air (`CreateDamageSources.fanLava`, 4).
pub const BLASTING_DAMAGE: f32 = 4.0;
/// Ticks an entity burns for in smoke (`igniteForSeconds(2)`).
pub const SMOKING_FIRE_TICKS: i32 = 40;
/// Damage in smoke (`CreateDamageSources.fanFire`, 2).
pub const SMOKING_DAMAGE: f32 = 2.0;
/// Haunting blinds for 30 ticks and slows (amplifier 1) for 20, with no particles.
pub const HAUNTING_BLINDNESS: (i32, u8) = (30, 0);
pub const HAUNTING_SLOWNESS: (i32, u8) = (20, 1);
/// Splashing hurts these by drowning for 2 (`entity.damageSources().drown()`).
pub fn splashing_hurts(entity_kind: &str) -> bool {
    matches!(
        entity_kind,
        "minecraft:enderman" | "minecraft:snow_golem" | "minecraft:blaze"
    )
}

// --- the Wrench (`IWrenchable.getRotatedBlockState`) ------------------------------------------

/// `Direction.getClockWise(Axis)`: `facing` a quarter turn clockwise seen from the positive end
/// of `axis`; unchanged when it lies along the axis.
pub fn clockwise(facing: Dir, axis: Axis) -> Dir {
    match axis {
        Axis::Y => match facing {
            Dir::North => Dir::East,
            Dir::East => Dir::South,
            Dir::South => Dir::West,
            Dir::West => Dir::North,
            other => other,
        },
        // `getClockWiseX`
        Axis::X => match facing {
            Dir::Up => Dir::South,
            Dir::North => Dir::Up,
            Dir::South => Dir::Down,
            Dir::Down => Dir::North,
            other => other,
        },
        // `getClockWiseZ`
        Axis::Z => match facing {
            Dir::Up => Dir::East,
            Dir::East => Dir::Down,
            Dir::Down => Dir::West,
            Dir::West => Dir::Up,
            other => other,
        },
    }
}

/// The fan's `facing` after a Wrench click on `clicked` face: it turns round that face's axis,
/// and stays when it points along it.
pub fn wrench_facing(facing: Dir, clicked: Dir) -> Dir {
    if facing.axis() == clicked.axis() {
        facing
    } else {
        clockwise(facing, clicked.axis())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrench_turns_the_fan_round_the_clicked_face() {
        assert_eq!(wrench_facing(Dir::North, Dir::Up), Dir::East);
        assert_eq!(wrench_facing(Dir::West, Dir::Up), Dir::North);
        assert_eq!(wrench_facing(Dir::Up, Dir::East), Dir::South);
        assert_eq!(wrench_facing(Dir::Up, Dir::Up), Dir::Up);
        assert_eq!(wrench_facing(Dir::East, Dir::South), Dir::Down);
        let mut f = Dir::North;
        for _ in 0..4 {
            f = wrench_facing(f, Dir::East);
        }
        assert_eq!(f, Dir::North);
    }

    #[test]
    fn reach_grows_with_speed() {
        assert_eq!(max_distance(0.0), 3.0);
        assert_eq!(max_distance(256.0), 20.0);
        assert_eq!(max_distance(-512.0), 20.0);
        // 3 + 128/256 * 17 = 11.5
        assert_eq!(max_distance(128.0), 11.5);
        assert_eq!(max_distance(-128.0), 11.5);
    }

    #[test]
    fn the_air_follows_the_speed_through_the_facing() {
        assert_eq!(flow_direction(0.0, Dir::Up), None);
        assert_eq!(flow_direction(16.0, Dir::Up), Some(Dir::Up));
        assert_eq!(flow_direction(-16.0, Dir::Up), Some(Dir::Down));
        // A facing on the negative end of its axis reads the speed the other way round.
        assert_eq!(flow_direction(16.0, Dir::North), Some(Dir::South));
        assert_eq!(flow_direction(-16.0, Dir::North), Some(Dir::North));
        assert!(pushing(-16.0, Dir::North));
        assert!(!pushing(16.0, Dir::North));
        assert!(pushing(16.0, Dir::East));
    }

    fn boxes(list: &[Aabb]) -> Passage {
        Passage::Boxes(list.to_vec())
    }

    #[test]
    fn open_air_reaches_the_full_distance() {
        assert_eq!(flow_limit(20.0, Dir::East, |_| boxes(&[])), 20.0);
    }

    #[test]
    fn a_full_block_stops_the_air_in_front_of_it() {
        let full = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let at = |n: i32| move |i: i32| if i == n { boxes(&[full]) } else { boxes(&[]) };
        assert_eq!(flow_limit(20.0, Dir::East, at(0)), 0.0);
        assert_eq!(flow_limit(20.0, Dir::East, at(6)), 6.0);
    }

    #[test]
    fn transparent_blocks_and_unloaded_ground() {
        let full = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert_eq!(flow_limit(10.0, Dir::East, |i| if i < 3 { Passage::Transparent } else { boxes(&[full]) }), 3.0);
        assert_eq!(flow_limit(10.0, Dir::East, |i| if i < 4 { Passage::Transparent } else { Passage::Unloaded }), 4.0);
    }

    #[test]
    fn a_slab_lets_the_air_in_by_its_depth() {
        let bottom_slab = [0.0, 0.0, 0.0, 1.0, 0.5, 1.0];
        // Blowing sideways over a bottom slab: the samples above it find nothing, so the air
        // goes on.
        assert_eq!(shape_depth(&[bottom_slab], Dir::East), f64::INFINITY);
        assert_eq!(flow_limit(20.0, Dir::East, |i| if i == 2 { boxes(&[bottom_slab]) } else { boxes(&[]) }), 20.0);
        // Blowing up into it, it is 0 deep from below and the air stops 1/32 past its block's
        // near face; blowing down onto it, it is half deep.
        assert_eq!(shape_depth(&[bottom_slab], Dir::Up), 0.0);
        assert_eq!(shape_depth(&[bottom_slab], Dir::Down), 0.5);
        assert_eq!(flow_limit(20.0, Dir::Down, |i| if i == 2 { boxes(&[bottom_slab]) } else { boxes(&[]) }), 2.5 + 1.0 / 32.0);
    }

    #[test]
    fn a_hole_in_the_shape_lets_the_air_through() {
        // A frame with an empty middle: the centre sample finds nothing.
        let frame = [
            [0.0, 0.0, 0.0, 1.0, 1.0, 0.25],
            [0.0, 0.0, 0.75, 1.0, 1.0, 1.0],
            [0.0, 0.0, 0.25, 0.25, 1.0, 0.75],
            [0.75, 0.0, 0.25, 1.0, 1.0, 0.75],
        ];
        assert_eq!(shape_depth(&frame, Dir::Up), f64::INFINITY);
        assert_eq!(flow_limit(5.0, Dir::Up, |_| boxes(&frame)), 5.0);
    }

    #[test]
    fn a_post_is_a_hole_for_the_samples_beside_it() {
        // A fence post, 6 to 10 texels: the quarter points miss it.
        let post = [0.375, 0.0, 0.375, 0.625, 1.0, 0.625];
        assert_eq!(shape_depth(&[post], Dir::East), f64::INFINITY);
    }

    #[test]
    fn a_stair_has_a_hole_and_stacked_halves_have_none() {
        // A bottom stair seen from the side: the high quarter is empty, so one sample misses.
        let stair = [[0.0, 0.0, 0.0, 1.0, 0.5, 1.0], [0.0, 0.0, 0.0, 1.0, 1.0, 0.5]];
        assert_eq!(shape_depth(&stair, Dir::East), f64::INFINITY);
        // Cells are `[low, high)`: two halves meeting at 0.5 cover the sample at 0.5 once.
        let halves = [[0.0, 0.0, 0.0, 1.0, 0.5, 1.0], [0.0, 0.5, 0.0, 1.0, 1.0, 1.0]];
        assert_eq!(shape_depth(&halves, Dir::East), 0.0);
    }

    #[test]
    fn the_limit_counts_a_partial_block() {
        assert_eq!(limit(20.0), 20);
        assert_eq!(limit(11.5), 12);
        assert_eq!(limit(0.0), 0);
    }

    #[test]
    fn the_bounds_run_from_the_fans_face() {
        assert_eq!(bounds([0, 0, 0], Dir::East, 0.2), None);
        assert_eq!(bounds([5, 64, 5], Dir::East, 3.0), Some(([6.0, 64.0, 5.0], [9.0, 65.0, 6.0])));
        assert_eq!(bounds([5, 64, 5], Dir::North, 2.5), Some(([5.0, 64.0, 2.5], [6.0, 65.0, 5.0])));
        assert_eq!(bounds([5, 64, 5], Dir::Up, 0.5), Some(([5.0, 65.0, 5.0], [6.0, 65.5, 6.0])));
        assert_eq!(bounds([5, 64, 5], Dir::Down, 4.0), Some(([5.0, 60.0, 5.0], [6.0, 64.0, 6.0])));
    }

    #[test]
    fn catalysts_follow_create_s_rules() {
        let none = Cell::default();
        assert_eq!(none.kind(), None);
        let water = Cell { water: true, ..none };
        assert_eq!(water.kind(), Some(Kind::Splashing));
        let lava = Cell { lava: true, ..none };
        assert_eq!(lava.kind(), Some(Kind::Blasting));
        let fire = Cell { in_smoking: true, ..none };
        assert_eq!(fire.kind(), Some(Kind::Smoking));
        let soul_fire = Cell { in_haunting: true, ..none };
        assert_eq!(soul_fire.kind(), Some(Kind::Haunting));
        // An unlit campfire does nothing.
        let campfire = Cell { in_smoking: true, campfire: true, lit: Some(true), ..none };
        assert_eq!(campfire.kind(), Some(Kind::Smoking));
        let out = Cell { lit: Some(false), ..campfire };
        assert_eq!(out.kind(), None);
        let soul_campfire = Cell { in_haunting: true, campfire: true, lit: Some(true), ..none };
        assert_eq!(soul_campfire.kind(), Some(Kind::Haunting));
        assert_eq!(Cell { lit: Some(false), ..soul_campfire }.kind(), None);
    }

    #[test]
    fn blaze_burners_heat_the_air_by_their_level() {
        let burner = |heat| Cell { in_smoking: true, in_blasting: true, heat: Some(heat), ..Cell::default() };
        assert_eq!(burner(Heat::None).kind(), None);
        assert_eq!(burner(Heat::Smouldering).kind(), Some(Kind::Smoking));
        assert_eq!(burner(Heat::Fading).kind(), Some(Kind::Blasting));
        assert_eq!(burner(Heat::Kindled).kind(), Some(Kind::Blasting));
        assert_eq!(burner(Heat::Seething).kind(), Some(Kind::Blasting));
        let lit = |flame| Cell { in_smoking: true, in_haunting: true, flame: Some(flame), ..Cell::default() };
        assert_eq!(lit(Flame::Regular).kind(), Some(Kind::Smoking));
        assert_eq!(lit(Flame::Soul).kind(), Some(Kind::Haunting));
    }

    #[test]
    fn water_beats_everything_and_haunting_beats_smoking() {
        let all = Cell { water: true, lava: true, in_haunting: true, in_smoking: true, ..Cell::default() };
        assert_eq!(all.kind(), Some(Kind::Splashing));
        let fire = Cell { lava: true, in_haunting: true, in_smoking: true, ..Cell::default() };
        assert_eq!(fire.kind(), Some(Kind::Haunting));
    }

    #[test]
    fn water_is_found_in_waterlogged_blocks_and_plants() {
        assert!(holds_water("minecraft:water", false));
        assert!(holds_water("minecraft:oak_slab", true));
        assert!(holds_water("minecraft:kelp", false));
        assert!(!holds_water("minecraft:stone", false));
    }

    #[test]
    fn the_type_carries_on_until_another_is_found() {
        // Pushing, water at block 3 of 6: blocks 1 and 2 are plain air.
        let kind_at = |i: i32| (i == 3).then_some(Kind::Splashing);
        let found = segments(6.0, true, kind_at);
        assert_eq!(
            found,
            vec![
                Segment { kind: None, start: 0, end: 2 },
                Segment { kind: Some(Kind::Splashing), start: 2, end: 6 },
            ]
        );
        assert_eq!(kind_at_offset(&found, 6.0, true, 1.0), None);
        assert_eq!(kind_at_offset(&found, 6.0, true, 2.0), None);
        assert_eq!(kind_at_offset(&found, 6.0, true, 2.5), Some(Kind::Splashing));
        assert_eq!(kind_at_offset(&found, 6.0, true, 6.5), None);
    }

    #[test]
    fn a_pulling_fan_reads_the_current_from_its_far_end() {
        // Pulling from 6 blocks away: lava at block 5 reaches everything nearer.
        let found = segments(6.0, false, |i| (i == 5).then_some(Kind::Blasting));
        assert_eq!(
            found,
            vec![
                Segment { kind: None, start: 6, end: 5 },
                Segment { kind: Some(Kind::Blasting), start: 5, end: 0 },
            ]
        );
        assert_eq!(kind_at_offset(&found, 6.0, false, 5.5), None);
        assert_eq!(kind_at_offset(&found, 6.0, false, 4.0), Some(Kind::Blasting));
    }

    #[test]
    fn nothing_is_in_the_air_with_no_reach() {
        assert!(segments(0.0, true, |_| None).is_empty());
    }

    #[test]
    fn distance_is_taken_from_the_fans_face() {
        assert_eq!(aligned_distance_to_face([5, 64, 5], Dir::East, [8.5, 64.0, 5.5]), 2.5);
        assert_eq!(aligned_distance_to_face([5, 64, 5], Dir::West, [2.0, 64.0, 5.5]), 3.0);
        assert_eq!(aligned_distance_to_face([5, 64, 5], Dir::Up, [5.5, 67.0, 5.5]), 2.0);
    }

    #[test]
    fn the_current_pulls_an_entity_to_its_speed_gradually() {
        // 256 RPM, 10 blocks from a fan reaching 20: 256/512 / (10/20) = 1 block a tick, an
        // eighth of the way each tick.
        let v = push_velocity([0.0; 3], Dir::East, 256.0, false, 10.0, 20.0);
        assert!((v[0] - 0.125).abs() < 1e-9);
        assert_eq!(v[1], 0.0);
        // Gravity is damped, not added to.
        let v = push_velocity([0.0, -0.8, 0.0], Dir::East, 256.0, false, 10.0, 20.0);
        assert!((v[1] + 0.7).abs() < 1e-9);
        // Sneaking: eight times weaker.
        let v = push_velocity([0.0; 3], Dir::East, 256.0, true, 10.0, 20.0);
        assert!((v[0] - 0.015625).abs() < 1e-9);
        // The sign of the speed doesn't matter: the caller picks the direction.
        let v = push_velocity([0.0; 3], Dir::West, -256.0, false, 10.0, 20.0);
        assert!((v[0] + 0.125).abs() < 1e-9);
        // At the very middle of the fan the push stays finite.
        assert!(push_velocity([0.0; 3], Dir::Up, 64.0, false, 0.0, 10.0)[1].is_finite());
    }

    #[test]
    fn the_push_is_clamped_per_tick() {
        let v = push_velocity([0.0; 3], Dir::East, 256.0, false, 0.001, 20.0);
        assert_eq!(v[0], 5.0 / 8.0);
    }

    #[test]
    fn a_stack_takes_150_ticks_per_sixteen_items() {
        assert_eq!(processing_time(1), 151);
        assert_eq!(processing_time(16), 151);
        assert_eq!(processing_time(17), 301);
        assert_eq!(processing_time(64), 601);
    }

    #[test]
    fn processing_counts_down_to_the_result() {
        let mut progress = Progress::default();
        assert_eq!(can_process(None, Kind::Splashing), Known::Ask);
        let mut ticks = 0;
        loop {
            ticks += 1;
            if decrement(&mut progress, Kind::Splashing, 4) == 0 {
                break;
            }
        }
        assert_eq!(ticks, 150 + 1);
        // Done: it may be asked again once, then it counts below zero and stops.
        assert_eq!(can_process(Some(&progress), Kind::Splashing), Known::Yes);
        assert_eq!(decrement(&mut progress, Kind::Splashing, 4), -1);
        assert_eq!(can_process(Some(&progress), Kind::Splashing), Known::No);
        // Another type starts over.
        assert_eq!(can_process(Some(&progress), Kind::Haunting), Known::Ask);
        assert_eq!(decrement(&mut progress, Kind::Haunting, 4), 150);
        assert_eq!(progress.kind.as_deref(), Some("create:haunting"));
    }

    #[test]
    fn legacy_names_parse() {
        assert_eq!(Kind::parse("create:splashing"), Some(Kind::Splashing));
        assert_eq!(Kind::parse("BLASTING"), Some(Kind::Blasting));
        assert_eq!(Kind::parse("blasting"), None);
        assert_eq!(Kind::parse(""), None);
    }
}
