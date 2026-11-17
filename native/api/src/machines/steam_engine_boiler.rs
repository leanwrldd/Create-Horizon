//! The boiler (`BoilerData`, `BoilerHeaters`, `FluidTankRenderer.renderAsBoiler`): a Fluid Tank
//! becomes one when a Steam Engine or a Steam Whistle is attached to it. Its numbers (heat, size,
//! water, the share each engine gets) are `create_kinetics::rules::steam_engine`; this is the glue,
//! saved under the controller's `Boiler` key as Create saves it (`Supply`, `ActiveHeat`,
//! `PassiveHeat`, `Engines`, `Whistles`, `Update`).
//!
//! An engine or a whistle reports itself to its tank's controller every tick (`register`) and the
//! controller counts the reports, instead of Create's scan of every block around the tank when
//! something is placed or removed (`BoilerData.evaluate`). While a boiler is active it stops being
//! a tank: it keeps no fluid, takes water only (`BoilerFluidHandler`: each millibucket is counted
//! as supply and thrown away), its windows go plain and can't be toggled, and a gauge shows on
//! its sides.
//!
//! Known differences from Create (for the README):
//! - The heat under the tank is read again every 5 ticks, not when the block below changes
//!   (`FluidTankBlock.updateShape`); a change shows up to a quarter second late. An engine or
//!   whistle placed or broken counts a tick or two late too (`updateBoilerState`).
//! - The pipe organ and Full Steam advancements and the steam hiss of
//!   a working engine (`BoilerData.sound`) are not done.
//! - The gauge needle jumps to its value (Create's `LerpedFloat` chases it and jitters above 1),
//!   and the gauges' occlusion is looked at every 10 ticks as in `updateOcclusion`, from the
//!   client's collision boxes.
//! - A bucket on a boiler does nothing (Create's goes through the boiler handler); only pipes and
//!   pumps feed it. The Creative Fluid Tank is a boiler with all the water it needs but, as in
//!   Create, its engines don't run (`SteamEngineBlockEntity.isValid` wants `create:fluid_tank`).
//! - The boiler's sampled water isn't saved (as in Create): it starts from the saved supply.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use create_kinetics::rules::basin::HeatLevel;
use create_kinetics::rules::steam_engine::{self as rules, Boiler, Sampler};
use create_kinetics::{Axis, Dir};
use horizon_client::mod_api::{
    BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent, Compound, Hooks, Host,
    ModBlockEntity, StandaloneModelsEvent, Tag,
};

use super::fluid_tank::{self, BoilerApi};
use super::fluids::{CREATIVE_FLUID_TANK, FLUID_TANK};
use crate::util;

type Pos = [i32; 3];

/// `AllPartialModels.BOILER_GAUGE` and `BOILER_GAUGE_DIAL`.
const GAUGE: &str = "create:block/steam_engine/gauge";
const GAUGE_DIAL: &str = "create:block/steam_engine/gauge_dial";
const BLAZE_BURNER: &str = "create:blaze_burner";
/// `AllTags.AllBlockTags.PASSIVE_BOILER_HEATERS`.
const PASSIVE_HEATERS: &str = "create:passive_boiler_heaters";

// `BoilerData.write`'s keys.
const BOILER: &str = "Boiler";
const SUPPLY: &str = "Supply";
const ACTIVE_HEAT: &str = "ActiveHeat";
const PASSIVE_HEAT: &str = "PassiveHeat";
const ENGINES: &str = "Engines";
const WHISTLES: &str = "Whistles";
const UPDATE: &str = "Update";
// `FluidTankBlockEntity.write`'s keys.
const CONTROLLER: &str = "Controller";
const SIZE: &str = "Size";
const HEIGHT: &str = "Height";

/// How often the heat under the tank is read again, in ticks.
const HEAT_POLL: i64 = 5;
/// How long a report from an engine or whistle counts, in ticks.
const REPORT_TICKS: i64 = 2;
/// `BoilerData.updateOcclusion` runs from `lazyTick`, every 10 ticks.
const OCCLUSION_TICKS: i64 = 10;
/// How long a freshly loaded boiler waits for its engines to report.
const GRACE_TICKS: i64 = 10;

struct Boilers;

pub fn hooks() -> Box<dyn Hooks> {
    // The tank module asks the boiler through these, so it doesn't depend on this file.
    let _ = fluid_tank::BOILER.set(BoilerApi {
        active,
        fill,
        update_tag,
        capacity: rules::HANDLER_CAPACITY,
    });
    Box::new(Boilers)
}

pub fn is_tank(block: &str) -> bool {
    block == FLUID_TANK || block == CREATIVE_FLUID_TANK
}

// --- what an engine or a whistle tells its tank -----------------------------------------------------

/// What attaches to a tank.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attached {
    Engine,
    Whistle,
}

type Reports = HashMap<(String, Pos), HashMap<Pos, (Attached, i64)>>;
static REPORTS: LazyLock<Mutex<Reports>> = LazyLock::new(Mutex::default);
/// When a loaded boiler first found no engine reporting.
static FIRST_SEEN: LazyLock<Mutex<HashMap<(String, Pos), i64>>> = LazyLock::new(Mutex::default);
static SAMPLERS: LazyLock<Mutex<HashMap<(String, Pos), (Sampler, i64)>>> = LazyLock::new(Mutex::default);
/// The gauges' occlusion, by tank, as last worked out: `[south, west, north, east]` and the game
/// time of it (client).
static OCCLUSION: LazyLock<Mutex<HashMap<Pos, ([bool; 4], i64)>>> = LazyLock::new(Mutex::default);

fn reports<R>(f: impl FnOnce(&mut Reports) -> R) -> R {
    f(&mut REPORTS.lock().unwrap_or_else(PoisonError::into_inner))
}

fn samplers_first(key: &(String, Pos), now: i64) -> i64 {
    *FIRST_SEEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(key.clone())
        .or_insert(now)
}

fn samplers<R>(f: impl FnOnce(&mut HashMap<(String, Pos), (Sampler, i64)>) -> R) -> R {
    f(&mut SAMPLERS.lock().unwrap_or_else(PoisonError::into_inner))
}

/// An engine or whistle at `at` stands on the tank whose controller is at `controller`: it counts
/// for the next two ticks (`BoilerData.evaluate`'s `attachedEngines` and `attachedWhistles`).
pub(crate) fn register(dimension: &str, controller: Pos, at: Pos, what: Attached, now: i64) {
    reports(|all| {
        all.entry((dimension.to_owned(), controller))
            .or_default()
            .insert(at, (what, now));
    });
}

/// The engines and whistles that reported lately to the controller at `controller`.
fn counts(dimension: &str, controller: Pos, now: i64) -> (i32, i32) {
    reports(|all| {
        let Some(at) = all.get_mut(&(dimension.to_owned(), controller)) else {
            return (0, 0);
        };
        at.retain(|_, (_, seen)| now - *seen < 40);
        let live = at.values().filter(|(_, seen)| now - *seen < REPORT_TICKS);
        let mut engines = 0;
        let mut whistles = 0;
        for (what, _) in live {
            match what {
                Attached::Engine => engines += 1,
                Attached::Whistle => whistles += 1,
            }
        }
        if at.is_empty() {
            all.remove(&(dimension.to_owned(), controller));
        }
        (engines, whistles)
    })
}

/// Whether anything has reported to the controller in this session (engines tick in their own
/// order, so a boiler just loaded hears from them a tick or two after it first ticks).
fn heard_from(dimension: &str, controller: Pos) -> bool {
    reports(|all| all.contains_key(&(dimension.to_owned(), controller)))
}

/// A tank's controller: where it is, and its block entity as the controller saves it. `size` is the
/// number of blocks the boiler is (`getTotalTankSize`).
pub(crate) struct Tank {
    pub controller: Pos,
    pub kind: String,
    pub boiler: Boiler,
    pub size: i32,
}

/// The controller of the tank block at `at`, if it is one (`FluidTankBlockEntity.getControllerBE`).
pub(crate) fn tank_at(host: &dyn Host, at: Pos) -> Option<Tank> {
    let entity = host.mod_block_entity(at)?;
    if !is_tank(&entity.id) {
        return None;
    }
    let controller = controller_pos(&entity.nbt).unwrap_or(at);
    let entity = if controller == at {
        entity
    } else {
        let entity = host.mod_block_entity(controller)?;
        if !is_tank(&entity.id) || controller_pos(&entity.nbt).is_some_and(|c| c != controller) {
            return None;
        }
        entity
    };
    Some(Tank {
        controller,
        kind: entity.id.clone(),
        boiler: read_boiler(&entity.nbt),
        size: size_of(&entity.nbt),
    })
}

fn controller_pos(nbt: &Compound) -> Option<Pos> {
    match util::get(nbt, CONTROLLER) {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn width_of(nbt: &Compound) -> i32 {
    util::int(nbt, SIZE).max(1)
}

/// `getTotalTankSize`: `width * width * height`.
fn size_of(nbt: &Compound) -> i32 {
    width_of(nbt) * width_of(nbt) * util::int(nbt, HEIGHT).max(1)
}

// --- the saved boiler -------------------------------------------------------------------------------------

/// `BoilerData.read`.
pub(crate) fn read_boiler(nbt: &Compound) -> Boiler {
    let Some(Tag::Compound(boiler)) = util::get(nbt, BOILER) else {
        return Boiler::default();
    };
    Boiler {
        water_supply: util::float(boiler, SUPPLY).unwrap_or(0.0),
        active_heat: util::int(boiler, ACTIVE_HEAT),
        passive_heat: util::boolean(boiler, PASSIVE_HEAT),
        engines: util::int(boiler, ENGINES),
        whistles: util::int(boiler, WHISTLES),
    }
}

/// `BoilerData.write`.
fn boiler_tag(boiler: &Boiler) -> Tag {
    let mut tag = Compound::new();
    tag.insert(SUPPLY.into(), Tag::Float(boiler.water_supply.to_bits()));
    tag.insert(ACTIVE_HEAT.into(), Tag::Int(boiler.active_heat));
    tag.insert(PASSIVE_HEAT.into(), Tag::Byte(i8::from(boiler.passive_heat)));
    tag.insert(ENGINES.into(), Tag::Int(boiler.engines));
    tag.insert(WHISTLES.into(), Tag::Int(boiler.whistles));
    tag.insert(UPDATE.into(), Tag::Byte(0));
    Tag::Compound(tag)
}

/// Writes the boiler into the controller's saved compound when it differs. True when it did.
fn save(entity: &mut ModBlockEntity, boiler: &Boiler) -> bool {
    let tag = boiler_tag(boiler);
    if !util::differs(&entity.nbt, BOILER, &tag) {
        return false;
    }
    entity.nbt.insert(BOILER.into(), tag);
    true
}

/// What the tank module asks of the boiler: whether the tank is a boiler now (`BoilerData.isActive`).
fn active(host: &dyn Host, controller: Pos) -> bool {
    host.mod_block_entity(controller)
        .is_some_and(|entity| is_tank(&entity.id) && read_boiler(&entity.nbt).is_active())
}

/// `BoilerFluidHandler.fill`: water is counted as supply and not kept. Answers the millibuckets
/// taken.
fn fill(host: &mut dyn Host, controller: Pos, fluid: &str, amount: i32, execute: bool) -> i32 {
    // `isFluidValid`: `FluidHelper.isWater`.
    if fluid != "minecraft:water" && fluid != "minecraft:flowing_water" {
        return 0;
    }
    if amount <= 0 {
        return 0;
    }
    if execute {
        let dimension = host.dimension().to_owned();
        let saved = host
            .mod_block_entity(controller)
            .map_or(0.0, |entity| read_boiler(&entity.nbt).water_supply);
        samplers(|all| {
            let (sampler, _) = all
                .entry((dimension, controller))
                .or_insert_with(|| (Sampler::from_supply(saved), 0));
            sampler.gather(amount);
        });
    }
    amount
}

/// The boiler's keys go to clients with the rest of the tank.
fn update_tag(entity: &ModBlockEntity, tag: &mut Compound) {
    if let Some(boiler) = util::get(&entity.nbt, BOILER) {
        tag.insert(BOILER.into(), boiler.clone());
    }
}

// --- heat -------------------------------------------------------------------------------------------------------

fn tagged(host: &dyn Host, block: &str, tag: &str, fallback: bool) -> bool {
    host.block_name_in_tag(block, tag).unwrap_or(fallback)
}

/// `BoilerHeater.findHeat` for the block at `at`.
fn heat_at(host: &dyn Host, at: Pos) -> i32 {
    let block = host.block(at);
    if block == "minecraft:air" {
        return rules::NO_HEAT;
    }
    // A Blaze Burner is registered directly (`BoilerHeaters.registerDefaults`).
    let burner = (block == BLAZE_BURNER).then(|| {
        host.block_property(at, "blaze")
            .as_deref()
            .and_then(HeatLevel::from_name)
            .unwrap_or(HeatLevel::None)
    });
    let campfire = tagged(host, &block, "minecraft:campfires", block.ends_with("campfire"));
    // `data/create/tags/block/passive_boiler_heaters.json`: lava, magma, campfires and fire; the
    // lit Blaze Burner is in Create's own tag too.
    let heater = tagged(
        host,
        &block,
        PASSIVE_HEATERS,
        campfire
            || matches!(
                block.as_str(),
                "create:lit_blaze_burner" | "minecraft:magma_block" | "minecraft:lava" | "minecraft:fire" | "minecraft:soul_fire"
            ),
    );
    // `BlockHelper.isNotUnheated`: an unlit campfire doesn't heat.
    let unheated = campfire && host.block_property(at, "lit").as_deref() == Some("false");
    rules::find_heat(burner, heater, unheated)
}

// --- the controller's tick -------------------------------------------------------------------------------------

fn tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let entity = &mut *event.entity;
    // Only the controller is the boiler.
    if controller_pos(&entity.nbt).is_some_and(|c| c != pos) {
        if util::has(&entity.nbt, BOILER) {
            util::remove(&mut entity.nbt, BOILER);
            host.block_entity_sync(pos);
        }
        return;
    }
    let dimension = host.dimension().to_owned();
    let key = (dimension.clone(), pos);
    let now = util::game_time(host);
    let creative = entity.id == CREATIVE_FLUID_TANK;
    let width = width_of(&entity.nbt);

    let (mut engines, mut whistles) = counts(&dimension, pos, now);
    let mut boiler = read_boiler(&entity.nbt);
    let was_active = boiler.is_active();
    // A boiler just loaded keeps its saved counts until its engines have had their first tick.
    let waiting = was_active && engines + whistles == 0 && !heard_from(&dimension, pos);
    if waiting {
        let first = samplers_first(&key, now);
        if now - first < GRACE_TICKS {
            (engines, whistles) = (boiler.engines, boiler.whistles);
        }
    } else {
        FIRST_SEEN.lock().unwrap_or_else(PoisonError::into_inner).remove(&key);
    }
    // `evaluate`: a new count asks for a look at the heat.
    let mut changed = boiler.set_attached(engines, whistles);

    if !boiler.is_active() {
        // Nothing attached: not a boiler, and no water is counted.
        samplers(|all| all.remove(&key));
        if changed && save(entity, &boiler) {
            host.block_entity_sync(pos);
        }
        return;
    }
    if !was_active {
        // `updateBoilerState`: a tank that becomes a boiler loses its windows.
        fluid_tank::set_windows(host, pos, entity, false);
    }

    // `needsHeatLevelUpdate`, polled.
    let spread = i64::from(pos[0] ^ pos[2]) + i64::from(pos[1]);
    if changed || !was_active || (now + spread).rem_euclid(HEAT_POLL) == 0 {
        let heats: Vec<i32> = (0..width)
            .flat_map(|x| (0..width).map(move |z| [pos[0] + x, pos[1] - 1, pos[2] + z]))
            .map(|at| heat_at(&*host, at))
            .collect();
        changed |= boiler.update_temperature(heats);
    }

    // The water: sampled every five ticks.
    let supply = boiler.water_supply;
    let sampled = samplers(|all| {
        let (sampler, seen) = all
            .entry(key.clone())
            .or_insert_with(|| (Sampler::from_supply(supply), now));
        *seen = now;
        let sampled = sampler.tick(&mut boiler.water_supply, creative);
        if now.rem_euclid(200) == 0 {
            all.retain(|_, (_, seen)| now - *seen < 400);
        }
        sampled
    });
    changed |= sampled && boiler.water_supply != supply;

    if (changed || !was_active) && save(entity, &boiler) {
        host.block_entity_sync(pos);
    }
}

// --- the gauges ----------------------------------------------------------------------------------------------------

/// `AngleHelper`'s `Direction.toYRot` for the four sides, in `get2DDataValue` order.
const SIDES: [(Dir, f32); 4] = [
    (Dir::South, 0.0),
    (Dir::West, 90.0),
    (Dir::North, 180.0),
    (Dir::East, 270.0),
];

/// `BoilerData.updateOcclusion` for one side: whether something stands where the gauge would hang.
fn occluded(host: &dyn Host, pos: Pos, width: i32, side: Dir) -> bool {
    let w = f64::from(width);
    let n = side.normal();
    let (step_x, step_z) = (f64::from(n[0]), f64::from(n[2]));
    // `new AABB(pos).move(w / 2 - .5, 0, w / 2 - .5).deflate(5 / 8)`: a quarter block about the
    // centre of the tank's foot; moved out of the face and widened along it.
    let centre = [
        f64::from(pos[0]) + w / 2.0,
        f64::from(pos[1]) + 0.5,
        f64::from(pos[2]) + w / 2.0,
    ];
    let reach = w / 2.0 + 0.25;
    let (cx, cz) = (centre[0] + step_x * reach, centre[2] + step_z * reach);
    let (grow_x, grow_y, grow_z) = (step_z.abs() / 2.0, 0.25, step_x.abs() / 2.0);
    let aabb = [
        cx - 0.125 - grow_x,
        centre[1] - 0.125 - grow_y,
        cz - 0.125 - grow_z,
        cx + 0.125 + grow_x,
        centre[1] + 0.125 + grow_y,
        cz + 0.125 + grow_z,
    ];
    // `Level.noCollision(aabb)`, from the blocks' collision boxes (a client can't ask the level).
    let cell = |v: f64| v.floor() as i32;
    for x in cell(aabb[0])..=cell(aabb[3]) {
        for y in cell(aabb[1])..=cell(aabb[4]) {
            for z in cell(aabb[2])..=cell(aabb[5]) {
                let origin = [f64::from(x), f64::from(y), f64::from(z)];
                let hit = host.block_collision([x, y, z]).iter().any(|b| {
                    (0..3).all(|i| b[i] + origin[i] < aabb[i + 3] && b[i + 3] + origin[i] > aabb[i])
                });
                if hit {
                    return true;
                }
            }
        }
    }
    false
}

fn occlusion(host: &dyn Host, pos: Pos, width: i32, now: i64) -> [bool; 4] {
    let cached = OCCLUSION
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&pos)
        .copied();
    if let Some((sides, at)) = cached
        && (0..OCCLUSION_TICKS).contains(&(now - at))
    {
        return sides;
    }
    let sides = SIDES.map(|(side, _)| occluded(host, pos, width, side));
    OCCLUSION
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(pos, (sides, now));
    sides
}

/// `FluidTankRenderer.renderAsBoiler`: a gauge and its needle on each free side.
fn draw_gauges(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let nbt = &event.entity.nbt;
    // Only the controller, and only when windows are off (`!be.window`) and the boiler is active.
    let window = !util::has(nbt, "Window") || util::boolean(nbt, "Window");
    if controller_pos(nbt).is_some_and(|c| c != event.pos) || window {
        return;
    }
    let boiler = read_boiler(nbt);
    if !boiler.is_active() {
        return;
    }
    let width = width_of(nbt);
    let progress = boiler.gauge_target(size_of(nbt));
    let free = occlusion(host, event.pos, width, event.draw.game_time);
    let w = width as f32;
    let (pivot_y, pivot_z) = (6.0 / 16.0, 8.0 / 16.0);
    let draw = &mut *event.draw;
    for ((_, to_y_rot), blocked) in SIDES.into_iter().zip(free) {
        if blocked {
            continue;
        }
        let y_rot = -to_y_rot - 90.0;
        for (model, dial) in [(GAUGE, false), (GAUGE_DIAL, true)] {
            draw.push();
            draw.translate([w / 2.0, 0.5, w / 2.0]);
            draw.rotate_about(Axis::Y.unit(), y_rot, [0.0; 3]);
            draw.translate([-0.5, -0.5, -0.5]);
            draw.translate([w / 2.0 - 6.0 / 16.0, 0.0, 0.0]);
            if dial {
                draw.translate([0.0, pivot_y, pivot_z]);
                draw.rotate_about(Axis::X.unit(), -145.0 * progress + 90.0, [0.0; 3]);
                draw.translate([0.0, -pivot_y, -pivot_z]);
            }
            host.draw_model(draw, model, 0xFF_FFFF);
            draw.pop();
        }
    }
}

impl Hooks for Boilers {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(GAUGE.to_owned());
        event.models.push(GAUGE_DIAL.to_owned());
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_tank(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if is_tank(&event.entity.id) {
            tick(host, event);
        }
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if is_tank(event.block) {
            draw_gauges(host, event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nbt_with(boiler: &Boiler) -> Compound {
        let mut nbt = Compound::new();
        nbt.insert(BOILER.into(), boiler_tag(boiler));
        nbt
    }

    #[test]
    fn the_boiler_round_trips_through_its_keys() {
        let boiler = Boiler {
            water_supply: 24.5,
            active_heat: 3,
            passive_heat: false,
            engines: 2,
            whistles: 1,
        };
        assert_eq!(read_boiler(&nbt_with(&boiler)), boiler);
        assert_eq!(read_boiler(&Compound::new()), Boiler::default());
        let passive = Boiler {
            passive_heat: true,
            ..Boiler::default()
        };
        assert_eq!(read_boiler(&nbt_with(&passive)), passive);
    }

    #[test]
    fn reports_count_for_two_ticks() {
        let dimension = "minecraft:overworld";
        register(dimension, [0, 0, 0], [1, 0, 0], Attached::Engine, 100);
        register(dimension, [0, 0, 0], [2, 0, 0], Attached::Engine, 101);
        register(dimension, [0, 0, 0], [0, 5, 0], Attached::Whistle, 101);
        assert_eq!(counts(dimension, [0, 0, 0], 101), (2, 1));
        assert_eq!(counts(dimension, [0, 0, 0], 102), (1, 1));
        assert_eq!(counts(dimension, [0, 0, 0], 103), (0, 0));
        assert_eq!(counts(dimension, [9, 9, 9], 103), (0, 0));
    }

    #[test]
    fn tanks_have_a_size() {
        let mut nbt = Compound::new();
        nbt.insert(SIZE.into(), Tag::Int(3));
        nbt.insert(HEIGHT.into(), Tag::Int(4));
        assert_eq!(size_of(&nbt), 36);
        assert_eq!(size_of(&Compound::new()), 1);
        assert_eq!(controller_pos(&nbt), None);
        nbt.insert(CONTROLLER.into(), Tag::IntArray(vec![1, 2, 3]));
        assert_eq!(controller_pos(&nbt), Some([1, 2, 3]));
    }
}
