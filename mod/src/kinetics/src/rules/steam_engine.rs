//! The rules of steam power: the boiler (`BoilerData`, `BoilerHeaters`), the Steam Engine
//! (`SteamEngineBlock`, `SteamEngineBlockEntity`, `SteamEngineRenderer`) and the Powered Shaft it
//! turns a shaft into (`PoweredShaftBlock`, `PoweredShaftBlockEntity`), ported from Create
//! (MIT, the Creators of Create) and checked against `mc1.21.1/dev` at 4148fd8.
//!
//! No game types: the glue (`native/api/src/machines/steam_engine*.rs`) owns the world. The
//! Steam Whistle's rules are in `steam_engine_whistle.rs`.
//!
//! A tank is a boiler while a Steam Engine or Steam Whistle is attached to it. Its **level** is
//! how many engines it can run at full power: the least of the heat the Blaze Burners under it
//! give (a Blaze Burner counts 1, a seething one 2, a smouldering one or any lit fire or magma is
//! a *passive* heat), the size of the tank (`size / 4`, at most 18) and the water it is given
//! (one level per 10 millibuckets a tick, at most 18). Each engine then turns its powered shaft at
//! 16 to 64 RPM with up to 1024 su of capacity per RPM (`CStress.setCapacity(1024.0)`), by the
//! share of the level it gets.

use super::basin::HeatLevel;
use crate::{Axis, Dir, Pos};

// --- constants ---------------------------------------------------------------------------------

/// `BoilerData.SAMPLE_RATE`: ticks between two samples of the water supplied.
pub const SAMPLE_RATE: i32 = 5;
/// The samples kept: `supplyOverTime = new float[10]`, so the supply is the best of 50 ticks.
pub const SAMPLES: usize = 10;
/// `BoilerData.waterSupplyPerLevel`: millibuckets a tick for each level.
pub const WATER_SUPPLY_PER_LEVEL: i32 = 10;
/// `BoilerData.passiveEngineEfficiency`: what a passively heated boiler gives its engines, shared.
pub const PASSIVE_ENGINE_EFFICIENCY: f32 = 1.0 / 8.0;
/// The highest boiler level (`Math.min(18, ...)` in `BoilerData`).
pub const MAX_LEVEL: i32 = 18;
/// What a creative tank supplies a boiler: `waterSupplyPerLevel * 20`.
pub const CREATIVE_SUPPLY: f32 = (WATER_SUPPLY_PER_LEVEL * 20) as f32;
/// `BoilerFluidHandler.getTankCapacity`.
pub const HANDLER_CAPACITY: i32 = 10_000;

/// `AllBlocks.STEAM_ENGINE`: `CStress.setCapacity(1024.0)`.
pub const ENGINE_CAPACITY: f32 = 1024.0;
/// The base speed of a powered shaft (`PoweredShaftBlockEntity.getGeneratedSpeed`: `16 * modifier`).
pub const BASE_SPEED: f32 = 16.0;
/// `PoweredShaftBlockEntity` starts with `initialTicks = 3`: it can't be powered before then.
pub const WARMUP_TICKS: i32 = 3;

/// `BoilerHeater.NO_HEAT`.
pub const NO_HEAT: i32 = -1;
/// `BoilerHeater.PASSIVE_HEAT`.
pub const PASSIVE_HEAT: i32 = 0;

// --- heat ----------------------------------------------------------------------------------------

/// `BoilerHeaters.blazeBurner`: the heat a Blaze Burner gives by its `blaze` state.
pub fn blaze_burner_heat(level: HeatLevel) -> i32 {
    if level == HeatLevel::None {
        NO_HEAT
    } else if level == HeatLevel::Seething {
        2
    } else if level.is_at_least(HeatLevel::Fading) {
        1
    } else {
        PASSIVE_HEAT
    }
}

/// `BoilerHeater.findHeat` for the block under a tank: a Blaze Burner (`burner` is its `blaze`
/// state, registered directly), else a block of `create:passive_boiler_heaters` that
/// `BlockHelper.isNotUnheated` doesn't call unlit (`BoilerHeaters.passive`), else none.
pub fn find_heat(burner: Option<HeatLevel>, passive_heater: bool, unheated: bool) -> i32 {
    if let Some(level) = burner {
        return blaze_burner_heat(level);
    }
    if passive_heater && !unheated {
        PASSIVE_HEAT
    } else {
        NO_HEAT
    }
}

// --- the boiler ----------------------------------------------------------------------------------

/// What `BoilerData` saves (`write` and `read`): the rest is derived or kept by [`Sampler`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Boiler {
    /// `waterSupply`: millibuckets a tick, the most of the last samples.
    pub water_supply: f32,
    /// `activeHeat`: the sum of the Blaze Burners' heat under the tank.
    pub active_heat: i32,
    /// `passiveHeat`: some block under the tank heats it passively and no burner does.
    pub passive_heat: bool,
    pub engines: i32,
    pub whistles: i32,
}

impl Boiler {
    /// `isActive`: a boiler is a tank with an engine or a whistle on it.
    pub fn is_active(&self) -> bool {
        self.engines > 0 || self.whistles > 0
    }

    /// `evaluate` after counting: the new numbers of engines and whistles. True when either
    /// changed (the caller then looks at the heat again, as `needsHeatLevelUpdate`).
    pub fn set_attached(&mut self, engines: i32, whistles: i32) -> bool {
        let changed = self.engines != engines || self.whistles != whistles;
        self.engines = engines;
        self.whistles = whistles;
        changed
    }

    /// `updateTemperature`: `heats` is `BoilerHeater.findHeat` of each block under the tank.
    /// True when the heat changed.
    pub fn update_temperature(&mut self, heats: impl IntoIterator<Item = i32>) -> bool {
        let (prev_passive, prev_active) = (self.passive_heat, self.active_heat);
        let mut passive = false;
        let mut active = 0;
        for heat in heats {
            if heat == PASSIVE_HEAT {
                passive = true;
            } else if heat > 0 {
                active += heat;
            }
        }
        self.passive_heat = passive && active == 0;
        self.active_heat = active;
        prev_active != self.active_heat || prev_passive != self.passive_heat
    }

    /// `getMaxHeatLevelForBoilerSize`: `(int) Math.min(18, boilerSize / 4)`, the size in blocks.
    pub fn max_level_for_size(size: i32) -> i32 {
        MAX_LEVEL.min(size / 4)
    }

    /// `getMaxHeatLevelForWaterSupply`: `Math.min(18, Mth.ceil(waterSupply) / 10)`.
    pub fn max_level_for_water(&self) -> i32 {
        MAX_LEVEL.min(self.water_supply.ceil() as i32 / WATER_SUPPLY_PER_LEVEL)
    }

    /// `isPassive(boilerSize)`: passively heated, with some size and some water.
    pub fn is_passive(&self, size: i32) -> bool {
        self.passive_heat && Self::max_level_for_size(size) > 0 && self.max_level_for_water() > 0
    }

    /// `getActualHeat`.
    pub fn actual_heat(&self, size: i32) -> i32 {
        self.active_heat
            .min(self.max_level_for_water().min(Self::max_level_for_size(size)))
    }

    /// `getEngineEfficiency`: the share of full power each engine gets, 0 to 1.
    pub fn engine_efficiency(&self, size: i32) -> f32 {
        if self.is_passive(size) {
            // Create divides by the engines, so none gives infinity; no engine asks then.
            return if self.engines > 0 {
                PASSIVE_ENGINE_EFFICIENCY / self.engines as f32
            } else {
                0.0
            };
        }
        if self.active_heat == 0 {
            return 0.0;
        }
        let actual = self.actual_heat(size);
        if self.engines <= actual {
            1.0
        } else {
            actual as f32 / self.engines as f32
        }
    }

    /// The level the goggles show (`getHeatLevelTextComponent`): the least of heat, water and size.
    pub fn level(&self, size: i32) -> i32 {
        self.active_heat
            .min(self.max_level_for_water().min(Self::max_level_for_size(size)))
    }

    /// The gauge's target, 0 to 1 (`BoilerData.read`): an eighth when passive.
    pub fn gauge_target(&self, size: i32) -> f32 {
        let for_size = Self::max_level_for_size(size);
        if self.is_passive(size) {
            1.0 / 8.0
        } else if for_size == 0 {
            0.0
        } else {
            self.actual_heat(size) as f32 / for_size as f32
        }
    }

    /// Whether a Steam Whistle sounds on this boiler (`WhistleBlockEntity.tick`, without the
    /// redstone): active and heated.
    pub fn whistles_sound(&self) -> bool {
        self.is_active() && (self.passive_heat || self.active_heat > 0)
    }

    /// The su the goggles say the boiler provides (`addToGoggleTooltip`): its efficiency times 16
    /// and the larger of its level and its engines, over the engine's capacity.
    pub fn total_stress(&self, size: i32) -> f32 {
        self.engine_efficiency(size)
            * 16.0
            * self.level(size).max(self.engines) as f32
            * ENGINE_CAPACITY
    }
}

/// The water a boiler is given, sampled as `BoilerData.tick` does: the millibuckets that came in
/// are counted over [`SAMPLE_RATE`] ticks and the boiler's supply is the best of the last
/// [`SAMPLES`] samples. Not saved by Create: a loaded boiler starts from its saved supply.
#[derive(Clone, Debug, PartialEq)]
pub struct Sampler {
    /// `gatheredSupply`.
    pub gathered: i32,
    /// `supplyOverTime`.
    pub over_time: [f32; SAMPLES],
    /// `ticksUntilNextSample`.
    pub until_next: i32,
    /// `currentIndex`.
    pub index: usize,
}

impl Sampler {
    /// `BoilerData.read`: `Arrays.fill(supplyOverTime, (int) waterSupply)`.
    pub fn from_supply(supply: f32) -> Self {
        Sampler {
            gathered: 0,
            over_time: [(supply as i32) as f32; SAMPLES],
            until_next: 0,
            index: 0,
        }
    }

    /// `BoilerFluidHandler.fill` for water: counts what came in.
    pub fn gather(&mut self, amount: i32) {
        self.gathered = self.gathered.saturating_add(amount);
    }

    /// One tick of `BoilerData.tick` after the heat: samples every [`SAMPLE_RATE`] ticks, updating
    /// `supply`. `creative` is a Creative Fluid Tank, which supplies [`CREATIVE_SUPPLY`]. True
    /// when a sample was taken (Create then sends the boiler to the clients).
    pub fn tick(&mut self, supply: &mut f32, creative: bool) -> bool {
        self.until_next -= 1;
        if self.until_next > 0 {
            return false;
        }
        self.until_next = SAMPLE_RATE;
        self.over_time[self.index] = self.gathered as f32 / SAMPLE_RATE as f32;
        *supply = supply.max(self.over_time[self.index]);
        self.index = (self.index + 1) % SAMPLES;
        self.gathered = 0;
        if self.index == 0 {
            *supply = 0.0;
            for sample in self.over_time {
                *supply = sample.max(*supply);
            }
        }
        if creative {
            *supply = CREATIVE_SUPPLY;
        }
        true
    }
}

// --- the Powered Shaft ----------------------------------------------------------------------------

/// `PoweredShaftBlockEntity.getSpeedModifier`: 4 at full efficiency, else 1 to 3 by quarters.
pub fn speed_modifier(efficiency: f32) -> i32 {
    (1.0 + if efficiency >= 1.0 {
        3.0
    } else {
        2.0f32.min((efficiency * 4.0).floor())
    }) as i32
}

/// `PoweredShaftBlockEntity.getCombinedCapacity`: the stress the engine adds at its efficiency.
pub fn combined_capacity(efficiency: f32, engine_capacity: f32) -> f32 {
    efficiency * engine_capacity
}

/// `PoweredShaftBlockEntity.getGeneratedSpeed`: `movementDirection * 16 * modifier` while there
/// is capacity.
pub fn generated_speed(direction: i32, efficiency: f32, engine_capacity: f32) -> f32 {
    if combined_capacity(efficiency, engine_capacity) > 0.0 {
        direction as f32 * BASE_SPEED * speed_modifier(efficiency) as f32
    } else {
        0.0
    }
}

/// `PoweredShaftBlockEntity.calculateAddedStressCapacity`: su per RPM.
pub fn added_capacity(efficiency: f32, engine_capacity: f32) -> f32 {
    combined_capacity(efficiency, engine_capacity) / speed_modifier(efficiency) as f32
}

/// `PoweredShaftBlockEntity.getRotationAngleOffset`: half a turn where the coordinate along the
/// axis is even (a Java `%`, so negative odd coordinates count as odd).
pub fn rotation_angle_offset(pos: Pos, axis: Axis) -> f32 {
    if pos[axis.index()] % 2 == 0 { 180.0 } else { 0.0 }
}

/// `KineticBlockEntityVisual.rotationOffset` for a shaft: 22.5 degrees where the other two
/// coordinates add up even.
pub fn position_offset(pos: Pos, axis: Axis) -> f32 {
    let sum: i32 = (0..3).filter(|i| *i != axis.index()).map(|i| pos[i]).sum();
    if sum % 2 == 0 { 22.5 } else { 0.0 }
}

/// `PoweredShaftBlockEntity.canBePoweredBy`: warmed up, and free or already this engine's.
pub fn can_be_powered_by(warmup: i32, engine: Option<Pos>, source: Pos, own: Pos) -> bool {
    warmup == 0
        && match engine {
            None => true,
            Some(key) => is_powered_by(own, Some(key), source),
        }
}

/// `PoweredShaftBlockEntity.isPoweredBy`: the key is the shaft's position minus the engine's.
pub fn is_powered_by(own: Pos, engine: Option<Pos>, source: Pos) -> bool {
    engine == Some([own[0] - source[0], own[1] - source[1], own[2] - source[2]])
}

// --- the Steam Engine -------------------------------------------------------------------------------

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`: the way the engine points,
/// at its shaft; its tank is behind it.
pub fn connected_direction(face: &str, facing: Dir) -> Dir {
    match face {
        "ceiling" => Dir::Down,
        "floor" => Dir::Up,
        _ => facing,
    }
}

/// `SteamEngineBlock.isShaftValid` for the kind of the shaft block: its axis must differ from
/// the engine's.
pub fn is_shaft_valid(connected: Dir, shaft_axis: Axis) -> bool {
    shaft_axis != connected.axis()
}

/// `PoweredShaftBlock.stillValid`: some engine two blocks away across the shaft's axis points at
/// it. `engine_at` answers the connected direction of the Steam Engine at an offset, if there is
/// one.
pub fn shaft_still_valid(shaft_axis: Axis, mut engine_at: impl FnMut(Pos) -> Option<Dir>) -> bool {
    for side in Dir::ALL {
        if side.axis() == shaft_axis {
            continue;
        }
        let n = side.normal();
        let Some(connected) = engine_at([n[0] * 2, n[1] * 2, n[2] * 2]) else {
            continue;
        };
        // `getShaftPos(engine) == pos`, and `isShaftValid`.
        if connected == side.opposite() && is_shaft_valid(connected, shaft_axis) {
            return true;
        }
    }
    false
}

/// What the engine asks of its shaft, from `SteamEngineBlockEntity.tick` before the check against
/// the network: `facing` is the connected direction, or the block's `facing` when that is
/// vertical. A vertical shaft always gets 1; with no efficiency the direction is 1; a Z shaft is
/// reversed, and so is an engine set to counter-clockwise.
pub fn conveyed_direction(
    efficiency: f32,
    target_axis: Axis,
    facing: Dir,
    counter_clockwise: bool,
) -> i32 {
    let mut level = if efficiency == 0.0 || target_axis == Axis::Y {
        1
    } else {
        facing.sign()
    };
    if target_axis == Axis::Z {
        level *= -1;
    }
    if counter_clockwise {
        level *= -1;
    }
    level
}

/// Whether the engine flips its own direction setting to match a network its shaft already
/// belongs to (`shaft.hasSource() && shaftSpeed != 0 && conveyed != 0 && sign mismatch`).
pub fn must_flip(has_source: bool, shaft_speed: f32, conveyed: i32) -> bool {
    has_source && shaft_speed != 0.0 && conveyed != 0 && (shaft_speed > 0.0) != (conveyed > 0)
}

/// `SteamEngineBlockEntity.getTargetAngle`: the shaft's angle (radians) as the engine's moving
/// parts see it, or none when the shaft lies along the engine's own axis.
pub fn target_angle(facing: Dir, shaft_axis: Axis, shaft_angle: f32) -> Option<f32> {
    let facing_axis = facing.axis();
    if shaft_axis == facing_axis {
        return None;
    }
    let mut angle = shaft_angle;
    if shaft_axis != Axis::Y && ((facing_axis == Axis::X) ^ (facing.sign() > 0)) {
        angle *= -1.0;
    }
    if shaft_axis == Axis::X && facing == Dir::Down {
        angle *= -1.0;
    }
    Some(angle)
}

/// The shaft's angle in radians (`KineticBlockEntityRenderer.getAngleForBe`): `time` in ticks with
/// the partial tick, `offset` the shaft's rotation offset in degrees.
pub fn shaft_angle(time: f64, rpm: f32, offset: f32) -> f32 {
    let degrees = (time * f64::from(rpm) * 3.0 / 10.0 + f64::from(offset)) % 360.0;
    (degrees / 180.0 * std::f64::consts::PI) as f32
}

/// `AngleHelper.horizontalAngle`: the facing's y rotation (south 0, west 90, north 180, east 270,
/// plus a half turn along x), 0 for a vertical one.
pub fn horizontal_angle(facing: Dir) -> f32 {
    let mut angle = match facing {
        Dir::South => 0.0,
        Dir::West => 90.0,
        Dir::North => 180.0,
        Dir::East => 270.0,
        Dir::Up | Dir::Down => return 0.0,
    };
    if facing.axis() == Axis::X {
        angle += 180.0;
    }
    angle
}

/// `AngleHelper.verticalAngle`.
pub fn vertical_angle(facing: Dir) -> f32 {
    match facing {
        Dir::Up => -90.0,
        Dir::Down => 90.0,
        _ => 0.0,
    }
}

/// `SteamEngineRenderer`'s `roll90`.
pub fn roll90(facing: Dir, shaft_axis: Axis) -> bool {
    let facing_axis = facing.axis();
    (facing_axis != Axis::Y && shaft_axis == Axis::Y) || (facing_axis == Axis::Y && shaft_axis == Axis::Z)
}

/// The piston's offset and the linkage's turn (radians) at `angle`, as `SteamEngineRenderer`
/// computes them from the crank's radius 6/16 and the rod's length 14/16.
pub fn piston_pose(angle: f32) -> (f32, f32) {
    let (sin, cos) = (angle.sin(), angle.cos());
    let piston = (6.0 / 16.0) * sin - ((14.0f32 / 16.0).powi(2) - (6.0f32 / 16.0).powi(2) * cos * cos).sqrt();
    let distance = (piston - 6.0 / 16.0 * sin).powi(2).sqrt();
    let angle2 = (distance / (14.0 / 16.0)).acos() * if cos >= 0.0 { 1.0 } else { -1.0 };
    (piston, angle2)
}

// --- placement ----------------------------------------------------------------------------------------

/// `Direction.orderedByNearest(entity)`: the six directions from the one the player looks
/// along most to the one they look away from most.
pub fn ordered_by_nearest(yaw: f32, pitch: f32) -> [Dir; 6] {
    let pitch_rad = f64::from(pitch) * (std::f64::consts::PI / 180.0);
    let yaw_rad = f64::from(-yaw) * (std::f64::consts::PI / 180.0);
    let (pitch_sin, pitch_cos) = (pitch_rad.sin(), pitch_rad.cos());
    let (yaw_sin, yaw_cos) = (yaw_rad.sin(), yaw_rad.cos());
    let x_pos = yaw_sin > 0.0;
    let y_pos = pitch_sin < 0.0;
    let z_pos = yaw_cos > 0.0;
    let x_yaw = if x_pos { yaw_sin } else { -yaw_sin };
    let y_mag = if y_pos { -pitch_sin } else { pitch_sin };
    let z_yaw = if z_pos { yaw_cos } else { -yaw_cos };
    let x_mag = x_yaw * pitch_cos;
    let z_mag = z_yaw * pitch_cos;
    let ax = if x_pos { Dir::East } else { Dir::West };
    let ay = if y_pos { Dir::Up } else { Dir::Down };
    let az = if z_pos { Dir::South } else { Dir::North };
    let make = |a: Dir, b: Dir, c: Dir| [a, b, c, c.opposite(), b.opposite(), a.opposite()];
    if x_yaw > z_yaw {
        if y_mag > x_mag {
            make(ay, ax, az)
        } else if z_mag > y_mag {
            make(ax, az, ay)
        } else {
            make(ax, ay, az)
        }
    } else if y_mag > z_mag {
        make(ay, az, ax)
    } else if x_mag > y_mag {
        make(az, ax, ay)
    } else {
        make(az, ay, ax)
    }
}

/// `Direction.fromYRot`: the horizontal direction a yaw faces.
pub fn from_yaw(yaw: f32) -> Dir {
    match (((f64::from(yaw) / 90.0 + 0.5).floor() as i64) & 3) as i32 {
        0 => Dir::South,
        1 => Dir::West,
        2 => Dir::North,
        _ => Dir::East,
    }
}

/// `BlockPlaceContext.getNearestLookingDirections`: the nearest-first order, with the face's
/// opposite moved to the front unless the click replaces the block clicked.
pub fn nearest_looking_directions(yaw: f32, pitch: f32, face: Dir, replace_clicked: bool) -> [Dir; 6] {
    let mut directions = ordered_by_nearest(yaw, pitch);
    if replace_clicked {
        return directions;
    }
    let target = face.opposite();
    if let Some(index) = directions.iter().position(|d| *d == target)
        && index > 0
    {
        directions.copy_within(0..index, 1);
        directions[0] = target;
    }
    directions
}

/// The state a Steam Engine is placed in (`FaceAttachedHorizontalDirectionalBlock.getStateForPlacement`
/// with `SteamEngineBlock.canSurvive`): the first direction the player looks along (nearest
/// first) that has a tank, `tank_at` says. A vertical look gives a floor or ceiling engine
/// facing the way the player does; a horizontal one gives a wall engine facing away from the tank.
/// Returns `(face, facing)`.
pub fn placement(
    yaw: f32,
    pitch: f32,
    face: Dir,
    replace_clicked: bool,
    mut tank_at: impl FnMut(Dir) -> bool,
) -> Option<(&'static str, Dir)> {
    for look in nearest_looking_directions(yaw, pitch, face, replace_clicked) {
        let (attach, facing) = if look.axis() == Axis::Y {
            (if look == Dir::Up { "ceiling" } else { "floor" }, from_yaw(yaw))
        } else {
            ("wall", look.opposite())
        };
        // The tank is behind the engine: `getConnectedDirection(state).getOpposite()`.
        if tank_at(connected_direction(attach, facing).opposite()) {
            return Some((attach, facing));
        }
    }
    None
}

/// `SteamEngineBlock.PlacementHelper`: the axis of the shaft placed by clicking the engine with
/// a shaft: the nearest direction's axis that `isShaftValid`.
pub fn helper_axis(yaw: f32, pitch: f32, connected: Dir) -> Axis {
    let mut axis = Axis::X;
    for look in ordered_by_nearest(yaw, pitch) {
        axis = look.axis();
        if is_shaft_valid(connected, axis) {
            break;
        }
    }
    axis
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boiler(heat: i32, water: f32, engines: i32) -> Boiler {
        Boiler {
            water_supply: water,
            active_heat: heat,
            passive_heat: false,
            engines,
            whistles: 0,
        }
    }

    #[test]
    fn blaze_burners_heat_by_level() {
        assert_eq!(blaze_burner_heat(HeatLevel::None), NO_HEAT);
        assert_eq!(blaze_burner_heat(HeatLevel::Smouldering), PASSIVE_HEAT);
        assert_eq!(blaze_burner_heat(HeatLevel::Fading), 1);
        assert_eq!(blaze_burner_heat(HeatLevel::Kindled), 1);
        assert_eq!(blaze_burner_heat(HeatLevel::Seething), 2);
    }

    #[test]
    fn other_blocks_heat_passively_when_lit() {
        assert_eq!(find_heat(None, true, false), PASSIVE_HEAT);
        assert_eq!(find_heat(None, true, true), NO_HEAT);
        assert_eq!(find_heat(None, false, false), NO_HEAT);
        // A burner is its own heater whatever the tag says.
        assert_eq!(find_heat(Some(HeatLevel::Seething), true, false), 2);
        assert_eq!(find_heat(Some(HeatLevel::None), true, false), NO_HEAT);
    }

    #[test]
    fn heat_adds_up_and_passive_only_counts_alone() {
        let mut b = Boiler::default();
        assert!(b.update_temperature([1, 2, NO_HEAT, PASSIVE_HEAT]));
        assert_eq!((b.active_heat, b.passive_heat), (3, false));
        assert!(!b.update_temperature([2, 1]));
        assert!(b.update_temperature([PASSIVE_HEAT, NO_HEAT]));
        assert_eq!((b.active_heat, b.passive_heat), (0, true));
        assert!(b.update_temperature([NO_HEAT]));
        assert_eq!((b.active_heat, b.passive_heat), (0, false));
    }

    #[test]
    fn the_boiler_level_is_the_least_of_heat_size_and_water() {
        // 4 tanks make level 1; 40 make 10; the cap is 18.
        assert_eq!(Boiler::max_level_for_size(3), 0);
        assert_eq!(Boiler::max_level_for_size(4), 1);
        assert_eq!(Boiler::max_level_for_size(40), 10);
        assert_eq!(Boiler::max_level_for_size(288), 18);
        // 10 mB a tick for each level; a fraction rounds up first.
        assert_eq!(boiler(0, 9.0, 0).max_level_for_water(), 0);
        assert_eq!(boiler(0, 10.0, 0).max_level_for_water(), 1);
        assert_eq!(boiler(0, 10.5, 0).max_level_for_water(), 1);
        assert_eq!(boiler(0, 11.0, 0).max_level_for_water(), 1);
        assert_eq!(boiler(0, 20.0, 0).max_level_for_water(), 2);
        assert_eq!(boiler(0, 1000.0, 0).max_level_for_water(), 18);
        assert_eq!(boiler(5, 30.0, 1).level(40), 3);
        assert_eq!(boiler(2, 30.0, 1).level(40), 2);
        assert_eq!(boiler(5, 30.0, 1).level(8), 2);
    }

    #[test]
    fn engines_share_the_level() {
        // Level 2 (two burners worth of heat, enough water and size) runs two engines fully.
        let size = 8;
        assert_eq!(boiler(2, 20.0, 1).engine_efficiency(size), 1.0);
        assert_eq!(boiler(2, 20.0, 2).engine_efficiency(size), 1.0);
        assert_eq!(boiler(2, 20.0, 4).engine_efficiency(size), 0.5);
        // No heat, no power.
        assert_eq!(boiler(0, 20.0, 2).engine_efficiency(size), 0.0);
        // Too little water: the water level caps it.
        assert_eq!(boiler(2, 10.0, 2).engine_efficiency(size), 0.5);
        // No size at all.
        assert_eq!(boiler(2, 20.0, 1).engine_efficiency(3), 0.0);
    }

    #[test]
    fn a_passive_boiler_gives_an_eighth_shared_between_its_engines() {
        let mut b = boiler(0, 20.0, 2);
        b.passive_heat = true;
        assert!(b.is_passive(4));
        assert_eq!(b.engine_efficiency(4), 1.0 / 16.0);
        b.engines = 1;
        assert_eq!(b.engine_efficiency(4), 1.0 / 8.0);
        // Not passive without water or size.
        b.water_supply = 0.0;
        assert!(!b.is_passive(4));
        assert_eq!(b.engine_efficiency(4), 0.0);
        b.water_supply = 20.0;
        assert!(!b.is_passive(3));
        // A burner's heat turns passive off (`passiveHeat &= activeHeat == 0`).
        let mut c = Boiler::default();
        c.update_temperature([PASSIVE_HEAT, 1]);
        assert!(!c.passive_heat);
    }

    #[test]
    fn the_gauge_and_goggles_follow_the_level() {
        let b = boiler(3, 30.0, 1);
        assert_eq!(b.gauge_target(48), 3.0 / 12.0);
        assert_eq!(b.gauge_target(3), 0.0);
        let mut p = boiler(0, 10.0, 1);
        p.passive_heat = true;
        assert_eq!(p.gauge_target(4), 0.125);
        // 16 su a RPM times the larger of level and engines, over 1024 capacity.
        assert_eq!(boiler(2, 20.0, 1).total_stress(8), 1.0 * 16.0 * 2.0 * 1024.0);
        assert_eq!(boiler(1, 10.0, 4).total_stress(8), 0.25 * 16.0 * 4.0 * 1024.0);
    }

    #[test]
    fn attached_engines_and_whistles_make_a_boiler() {
        let mut b = Boiler::default();
        assert!(!b.is_active());
        assert!(b.set_attached(0, 1));
        assert!(b.is_active());
        assert!(!b.set_attached(0, 1));
        assert!(b.set_attached(1, 1));
        assert!(b.set_attached(0, 0));
        assert!(!b.is_active());
    }

    #[test]
    fn whistles_need_heat() {
        let mut b = boiler(0, 0.0, 0);
        b.whistles = 1;
        assert!(!b.whistles_sound());
        b.passive_heat = true;
        assert!(b.whistles_sound());
        b.passive_heat = false;
        b.active_heat = 1;
        assert!(b.whistles_sound());
        b.whistles = 0;
        assert!(!b.whistles_sound());
    }

    #[test]
    fn water_is_sampled_over_fifty_ticks() {
        let mut sampler = Sampler::from_supply(0.0);
        let mut supply = 0.0;
        // The first tick samples at once (the counter starts at zero).
        sampler.gather(50);
        assert!(sampler.tick(&mut supply, false));
        assert_eq!(supply, 10.0);
        // Then every fifth tick.
        for _ in 0..4 {
            assert!(!sampler.tick(&mut supply, false));
        }
        sampler.gather(5);
        assert!(sampler.tick(&mut supply, false));
        // The best sample stays until the window wraps.
        assert_eq!(supply, 10.0);
        // After ten samples with nothing more, the window has the best of them (10) ...
        for _ in 0..8 {
            for _ in 0..4 {
                sampler.tick(&mut supply, false);
            }
            sampler.tick(&mut supply, false);
        }
        assert_eq!(sampler.index, 0);
        assert_eq!(supply, 10.0);
        // ... and ten empty samples later, none.
        for _ in 0..10 {
            for _ in 0..5 {
                sampler.tick(&mut supply, false);
            }
        }
        assert_eq!(supply, 0.0);
    }

    #[test]
    fn a_loaded_boiler_keeps_its_saved_supply() {
        let mut sampler = Sampler::from_supply(25.7);
        assert_eq!(sampler.over_time, [25.0; SAMPLES]);
        let mut supply = 25.7;
        // A full window of the saved value comes round before it can drop.
        for _ in 0..(SAMPLES * SAMPLE_RATE as usize) {
            sampler.tick(&mut supply, false);
        }
        assert_eq!(supply, 0.0);
    }

    #[test]
    fn a_creative_tank_supplies_enough_for_the_top_level() {
        let mut sampler = Sampler::from_supply(0.0);
        let mut supply = 0.0;
        assert!(sampler.tick(&mut supply, true));
        assert_eq!(supply, 200.0);
        let b = boiler(18, supply, 1);
        assert_eq!(b.max_level_for_water(), 18);
    }

    #[test]
    fn a_shaft_runs_at_16_to_64_by_the_engines_share() {
        assert_eq!(speed_modifier(1.0), 4);
        assert_eq!(speed_modifier(0.9), 3);
        assert_eq!(speed_modifier(0.5), 3);
        assert_eq!(speed_modifier(0.49), 2);
        assert_eq!(speed_modifier(0.25), 2);
        assert_eq!(speed_modifier(0.2), 1);
        assert_eq!(speed_modifier(0.0), 1);
        assert_eq!(generated_speed(1, 1.0, ENGINE_CAPACITY), 64.0);
        assert_eq!(generated_speed(-1, 1.0, ENGINE_CAPACITY), -64.0);
        assert_eq!(generated_speed(1, 0.3, ENGINE_CAPACITY), 32.0);
        assert_eq!(generated_speed(1, 0.1, ENGINE_CAPACITY), 16.0);
        assert_eq!(generated_speed(1, 0.0, ENGINE_CAPACITY), 0.0);
        // Capacity times speed is the share of 16384 su the engine gives.
        for efficiency in [1.0f32, 0.75, 0.5, 0.25, 0.125] {
            let total = added_capacity(efficiency, ENGINE_CAPACITY)
                * generated_speed(1, efficiency, ENGINE_CAPACITY).abs();
            assert!((total - efficiency * 16.0 * ENGINE_CAPACITY).abs() < 1e-2, "{efficiency}");
        }
    }

    #[test]
    fn the_shaft_is_offset_by_position() {
        assert_eq!(rotation_angle_offset([2, 0, 0], Axis::X), 180.0);
        assert_eq!(rotation_angle_offset([3, 0, 0], Axis::X), 0.0);
        assert_eq!(rotation_angle_offset([-3, 0, 0], Axis::X), 0.0);
        assert_eq!(rotation_angle_offset([-2, 5, 0], Axis::X), 180.0);
        assert_eq!(rotation_angle_offset([5, 4, 7], Axis::Y), 180.0);
        assert_eq!(position_offset([0, 5, 2], Axis::X), 0.0);
        assert_eq!(position_offset([0, 5, 3], Axis::X), 22.5);
        assert_eq!(position_offset([0, -3, 0], Axis::X), 0.0);
        assert_eq!(position_offset([7, 0, 9], Axis::Y), 22.5);
    }

    #[test]
    fn a_shaft_takes_one_engine_at_a_time() {
        let shaft = [0, 5, 0];
        let engine = [0, 3, 0];
        assert!(can_be_powered_by(0, None, engine, shaft));
        assert!(!can_be_powered_by(1, None, engine, shaft));
        assert!(can_be_powered_by(0, Some([0, 2, 0]), engine, shaft));
        assert!(!can_be_powered_by(0, Some([0, 2, 0]), [0, 4, 0], shaft));
        assert!(is_powered_by(shaft, Some([0, 2, 0]), engine));
        assert!(!is_powered_by(shaft, None, engine));
    }

    #[test]
    fn engines_point_at_their_shaft() {
        assert_eq!(connected_direction("floor", Dir::East), Dir::Up);
        assert_eq!(connected_direction("ceiling", Dir::East), Dir::Down);
        assert_eq!(connected_direction("wall", Dir::East), Dir::East);
        // A shaft may not lie along the engine's axis.
        assert!(is_shaft_valid(Dir::Up, Axis::X));
        assert!(is_shaft_valid(Dir::Up, Axis::Z));
        assert!(!is_shaft_valid(Dir::Up, Axis::Y));
        assert!(!is_shaft_valid(Dir::East, Axis::X));
        assert!(is_shaft_valid(Dir::East, Axis::Y));
    }

    #[test]
    fn a_powered_shaft_needs_an_engine_two_blocks_away() {
        // An engine above a floor tank at [0, -2, 0] ... its shaft is two above it.
        let below = |offset: Pos| (offset == [0, -2, 0]).then_some(Dir::Up);
        assert!(shaft_still_valid(Axis::X, below));
        assert!(shaft_still_valid(Axis::Z, below));
        // Along the engine's own axis: no.
        assert!(!shaft_still_valid(Axis::Y, below));
        // An engine that points elsewhere doesn't power it.
        let away = |offset: Pos| (offset == [0, -2, 0]).then_some(Dir::Down);
        assert!(!shaft_still_valid(Axis::X, away));
        let none = |_: Pos| None;
        assert!(!shaft_still_valid(Axis::X, none));
        // A wall engine east of the shaft points west at it.
        let east = |offset: Pos| (offset == [2, 0, 0]).then_some(Dir::West);
        assert!(shaft_still_valid(Axis::Y, east));
        assert!(shaft_still_valid(Axis::Z, east));
        assert!(!shaft_still_valid(Axis::X, east));
    }

    #[test]
    fn the_engine_picks_a_direction_by_the_shaft() {
        // Idle: always positive.
        assert_eq!(conveyed_direction(0.0, Axis::X, Dir::Down, false), 1);
        // A vertical shaft: positive.
        assert_eq!(conveyed_direction(1.0, Axis::Y, Dir::East, false), 1);
        // Along x or z the facing's sign, reversed for z.
        assert_eq!(conveyed_direction(1.0, Axis::X, Dir::Up, false), 1);
        assert_eq!(conveyed_direction(1.0, Axis::X, Dir::Down, false), -1);
        assert_eq!(conveyed_direction(1.0, Axis::Z, Dir::Up, false), -1);
        assert_eq!(conveyed_direction(1.0, Axis::Z, Dir::Down, false), 1);
        // Counter-clockwise reverses it; so does idle with a z shaft.
        assert_eq!(conveyed_direction(1.0, Axis::X, Dir::Up, true), -1);
        assert_eq!(conveyed_direction(0.0, Axis::Z, Dir::Up, false), -1);
    }

    #[test]
    fn the_engine_follows_a_network_that_turns_the_other_way() {
        assert!(must_flip(true, 16.0, -1));
        assert!(must_flip(true, -16.0, 1));
        assert!(!must_flip(true, 16.0, 1));
        assert!(!must_flip(false, 16.0, -1));
        assert!(!must_flip(true, 0.0, -1));
        assert!(!must_flip(true, 16.0, 0));
    }

    #[test]
    fn moving_parts_follow_the_shaft_angle() {
        let a = 0.7;
        // The shaft along the engine's axis: nothing moves.
        assert_eq!(target_angle(Dir::Up, Axis::Y, a), None);
        assert_eq!(target_angle(Dir::East, Axis::X, a), None);
        // Floor engine, horizontal shaft: facing up is on the y axis, which isn't x, so
        // `(false) ^ (true)` flips it.
        assert_eq!(target_angle(Dir::Up, Axis::Z, a), Some(-a));
        assert_eq!(target_angle(Dir::Up, Axis::X, a), Some(-a));
        // Ceiling engine facing down: `false ^ false`, kept; but x with down flips again.
        assert_eq!(target_angle(Dir::Down, Axis::Z, a), Some(a));
        assert_eq!(target_angle(Dir::Down, Axis::X, a), Some(-a));
        // A wall engine facing east (x axis, positive): `true ^ true` is false, kept.
        assert_eq!(target_angle(Dir::East, Axis::Y, a), Some(a));
        assert_eq!(target_angle(Dir::East, Axis::Z, a), Some(a));
        assert_eq!(target_angle(Dir::West, Axis::Z, a), Some(-a));
        assert_eq!(target_angle(Dir::North, Axis::Y, a), Some(a));
    }

    #[test]
    fn the_shaft_angle_runs_at_zero_point_three_degrees_a_tick_an_rpm() {
        let a = shaft_angle(100.0, 64.0, 0.0);
        let expected = (100.0 * 64.0 * 0.3 % 360.0) / 180.0 * std::f32::consts::PI;
        assert!((a - expected).abs() < 1e-4);
        assert!((shaft_angle(0.0, 16.0, 180.0) - std::f32::consts::PI).abs() < 1e-5);
    }

    #[test]
    fn facings_turn_the_models_as_create_does() {
        assert_eq!(horizontal_angle(Dir::South), 0.0);
        assert_eq!(horizontal_angle(Dir::West), 270.0);
        assert_eq!(horizontal_angle(Dir::North), 180.0);
        assert_eq!(horizontal_angle(Dir::East), 450.0);
        assert_eq!(horizontal_angle(Dir::Up), 0.0);
        assert_eq!(vertical_angle(Dir::Up), -90.0);
        assert_eq!(vertical_angle(Dir::Down), 90.0);
        assert_eq!(vertical_angle(Dir::North), 0.0);
        assert!(roll90(Dir::East, Axis::Y));
        assert!(roll90(Dir::Up, Axis::Z));
        assert!(!roll90(Dir::Up, Axis::X));
        assert!(!roll90(Dir::East, Axis::Z));
    }

    #[test]
    fn the_piston_swings_between_the_crank_and_the_rod() {
        // The crank at the top and the bottom of its turn.
        let (top, _) = piston_pose(std::f32::consts::FRAC_PI_2);
        let (bottom, _) = piston_pose(-std::f32::consts::FRAC_PI_2);
        assert!((top - (6.0 / 16.0 - 14.0 / 16.0)).abs() < 1e-5);
        assert!((bottom - (-6.0 / 16.0 - 14.0 / 16.0)).abs() < 1e-5);
        // The rod is straight along the piston when the crank is at the top or bottom.
        let (_, straight) = piston_pose(std::f32::consts::FRAC_PI_2);
        assert!(straight.abs() < 1e-2 || (straight - std::f32::consts::FRAC_PI_2).abs() < 1e-2);
        // The turn mirrors with the crank's side.
        let (_, left) = piston_pose(0.3);
        let (_, right) = piston_pose(std::f32::consts::PI - 0.3);
        assert!((left + right).abs() < 1e-4);
        // The piston stays within the stroke.
        for step in 0..64 {
            let angle = step as f32 / 64.0 * std::f32::consts::TAU;
            let (piston, _) = piston_pose(angle);
            assert!((-(6.0 + 14.0) / 16.0 - 1e-4..=-(14.0 - 6.0) / 16.0 + 1e-4).contains(&piston));
        }
    }

    #[test]
    fn looking_orders_the_directions_nearest_first() {
        // Yaw 0 looks south; pitch 0 is level.
        assert_eq!(ordered_by_nearest(0.0, 0.0)[0], Dir::South);
        assert_eq!(ordered_by_nearest(0.0, 0.0)[5], Dir::North);
        assert_eq!(ordered_by_nearest(90.0, 0.0)[0], Dir::West);
        assert_eq!(ordered_by_nearest(180.0, 0.0)[0], Dir::North);
        assert_eq!(ordered_by_nearest(-90.0, 0.0)[0], Dir::East);
        assert_eq!(ordered_by_nearest(0.0, 90.0)[0], Dir::Down);
        assert_eq!(ordered_by_nearest(0.0, -90.0)[0], Dir::Up);
        // Looking down at a slant: down first, then the way the player faces.
        let order = ordered_by_nearest(0.0, 60.0);
        assert_eq!(&order[..2], &[Dir::Down, Dir::South]);
        assert_eq!(from_yaw(0.0), Dir::South);
        assert_eq!(from_yaw(89.0), Dir::West);
        assert_eq!(from_yaw(-91.0), Dir::East);
        assert_eq!(from_yaw(180.0), Dir::North);
    }

    #[test]
    fn the_clicked_face_comes_first_unless_it_is_replaced() {
        let order = nearest_looking_directions(0.0, 0.0, Dir::North, false);
        // Clicking the north face of a block: the block is behind, to the south.
        assert_eq!(order[0], Dir::South);
        let order = nearest_looking_directions(0.0, 0.0, Dir::East, false);
        assert_eq!(order[0], Dir::West);
        assert_eq!(order[1], Dir::South);
        let order = nearest_looking_directions(0.0, 0.0, Dir::East, true);
        assert_eq!(order[0], Dir::South);
    }

    #[test]
    fn an_engine_is_placed_against_the_tank_the_player_looks_at() {
        // A tank below, looking down: a floor engine facing the way the player faces.
        let tank_below = |behind: Dir| behind == Dir::Down;
        assert_eq!(placement(0.0, 70.0, Dir::Up, false, tank_below), Some(("floor", Dir::South)));
        // A tank above: a ceiling engine.
        let tank_above = |behind: Dir| behind == Dir::Up;
        assert_eq!(placement(180.0, -70.0, Dir::Down, false, tank_above), Some(("ceiling", Dir::North)));
        // A tank to the east, the click on its west face (the player faces east): a wall engine
        // pointing away from the tank, so west... no: east of the engine is the tank, so the engine
        // faces west.
        let tank_east = |behind: Dir| behind == Dir::East;
        assert_eq!(placement(-90.0, 0.0, Dir::West, false, tank_east), Some(("wall", Dir::West)));
        // No tank in reach: refused.
        assert_eq!(placement(0.0, 0.0, Dir::North, false, |_| false), None);
        // Looking down but the only tank is at the side: it still attaches to the wall.
        let tank_north = |behind: Dir| behind == Dir::North;
        assert_eq!(placement(0.0, 50.0, Dir::Up, false, tank_north), Some(("wall", Dir::South)));
    }

    #[test]
    fn clicking_an_engine_with_a_shaft_picks_a_valid_axis() {
        // A floor engine (connected up): the shaft may lie along x or z, never y.
        assert_eq!(helper_axis(0.0, 0.0, Dir::Up), Axis::Z);
        assert_eq!(helper_axis(90.0, 0.0, Dir::Up), Axis::X);
        // Looking down: y is the nearest axis, so the next is taken.
        assert_ne!(helper_axis(0.0, 80.0, Dir::Up), Axis::Y);
        // A wall engine facing south takes x or y: a level look has nothing nearer than y after
        // south, so the tie goes to it.
        assert_eq!(helper_axis(0.0, 0.0, Dir::South), Axis::Y);
        assert_eq!(helper_axis(90.0, 0.0, Dir::South), Axis::X);
    }
}
