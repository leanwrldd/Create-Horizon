//! The Steam Engine and the Powered Shaft (`SteamEngineBlock`, `SteamEngineBlockEntity`,
//! `SteamEngineRenderer`, `PoweredShaftBlock`, `PoweredShaftBlockEntity`): an engine stands on the
//! side of a Fluid Tank and, with a Shaft two blocks in front of it, turns that shaft into a
//! Powered Shaft that generates rotation from the tank's boiler (`steam_engine_boiler.rs`). The
//! numbers (the boiler's share, the speed and capacity of the shaft, the geometry of the moving
//! parts) are `create_kinetics::rules::steam_engine`; this is the glue.
//!
//! The engine makes 16 to 64 RPM by the share of the boiler's level it gets, with 1024 su of
//! capacity per RPM at full power (`CStress.setCapacity(1024.0)`), so a full engine gives 16384 su.
//!
//! Known differences from Create (for the README):
//! - The direction setting (`ScrollOptionBehaviour`, `SteamEngineValueBox`) has no value box; it is
//!   kept under `ScrollValue` (0 clockwise, 1 counter-clockwise) and the engine still flips it to
//!   follow a network its shaft already belongs to. The Wrench's click on the top or bottom face
//!   turns a floor or ceiling engine, as `IWrenchable`'s default does.
//! - `onPlace` and `onRemove` are polled by the engine's tick: a Shaft in front of an engine
//!   (placed by hand, or by clicking the engine with a Shaft) becomes a Powered Shaft within a
//!   tick, and a Powered Shaft goes back to a Shaft within a tick of losing its engine. An engine
//!   whose tank is gone pops off (`canSurvive`) a tick later too; it doesn't while that chunk isn't
//!   loaded.
//! - Whether the shaft "has a source" (`KineticBlockEntity.hasSource`) is guessed from its speed
//!   against what it makes itself; a network stopped by overstress isn't counted as a source.
//! - Placing a Shaft by clicking the engine replaces air and fluids only (not every replaceable
//!   block), and plays the stone sound. The placement helper's ghost outline isn't shown.
//! - The steam particles (`SteamJetParticle`) are not done, and the hiss at each stroke (`BoilerData.sound`) plays once a turn of the shaft from the engine, not from the piston's drawn angle through a per-side sound pool; no Powerhouse advancement and no waterlogging.
//!   Goggles on an engine or its Powered
//!   Shaft show the capacity it provides (`goggles.rs`).
//! - A Powered Shaft can't take Shafts beside it by clicking (`PoweredShaftBlock.useItemOn`).

use create_kinetics::rules::sound_entries as rules_sound;
use create_kinetics::rules::steam_engine::{self as rules, ENGINE_CAPACITY};
use create_kinetics::{Axis, Dir, Node};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound,
    Flow, Hooks, Host, LevelHost, ModBlockEntity, StandaloneModelsEvent, StateForPlacementEvent,
    Tag, UseItemOnEvent,
};

use super::steam_engine_boiler::{self as boiler, Attached};
use crate::{kinetic, util};

type Pos = [i32; 3];

pub const ENGINE: &str = "create:steam_engine";
pub const POWERED_SHAFT: &str = "create:powered_shaft";
const SHAFT: &str = "create:shaft";
const CENTRE: [f32; 3] = [0.5; 3];

// `AllPartialModels.ENGINE_PISTON`, `ENGINE_LINKAGE` and `ENGINE_CONNECTOR`.
const PISTON: &str = "create:block/steam_engine/piston";
const LINKAGE: &str = "create:block/steam_engine/linkage";
const CONNECTOR: &str = "create:block/steam_engine/shaft_connector";

// `ScrollOptionBehaviour.write`'s key: 0 is `RotationDirection.CLOCKWISE`, 1 counter-clockwise.
const SCROLL: &str = "ScrollValue";
// `PoweredShaftBlockEntity.write`'s keys.
const DIRECTION: &str = "Direction";
const WARMUP: &str = "Warmup";
const ENGINE_POS: &str = "EnginePos";
const ENGINE_POWER: &str = "EnginePower";
const ENGINE_TYPE: &str = "EngineType";

struct Steam;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Steam)
}

fn add(a: Pos, b: Pos) -> Pos {
    util::add(a, b)
}

fn scaled(dir: Dir, by: i32) -> Pos {
    dir.normal().map(|v| v * by)
}

fn sub(a: Pos, b: Pos) -> Pos {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

// --- the engine's state ------------------------------------------------------------------------------

/// `SteamEngineBlock.getFacing`: the way the engine points, from its `face` and `facing`.
fn connected(face: Option<&str>, facing: Option<&str>) -> Option<Dir> {
    let facing = facing.and_then(Dir::from_name)?;
    Some(rules::connected_direction(face?, facing))
}

fn engine_state(host: &dyn Host, pos: Pos) -> Option<(Dir, Dir)> {
    let facing = host.block_property(pos, "facing")?;
    let face = host.block_property(pos, "face")?;
    let facing = Dir::from_name(&facing)?;
    Some((rules::connected_direction(&face, facing), facing))
}

/// The connected direction of the Steam Engine at `pos`, if that is one.
fn engine_at(host: &dyn Host, pos: Pos) -> Option<Dir> {
    if host.block(pos) != ENGINE {
        return None;
    }
    engine_state(host, pos).map(|(connected, _)| connected)
}

fn axis_of(host: &dyn Host, pos: Pos) -> Option<Axis> {
    host.block_property(pos, "axis").as_deref().and_then(Axis::from_name)
}

// --- the powered shaft's state ------------------------------------------------------------------------

/// What a Powered Shaft saves of its engine.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Driven {
    /// `enginePos`: the shaft's position minus the engine's.
    engine: Option<Pos>,
    /// `engineEfficiency`.
    power: f32,
    /// `movementDirection`.
    direction: i32,
    /// `initialTicks`.
    warmup: i32,
}

fn read_driven(nbt: &Compound) -> Driven {
    let engine = match util::get(nbt, ENGINE_POS) {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    };
    Driven {
        engine,
        power: if engine.is_some() {
            util::float(nbt, ENGINE_POWER).unwrap_or(0.0)
        } else {
            0.0
        },
        direction: util::int(nbt, DIRECTION),
        warmup: util::int(nbt, WARMUP),
    }
}

fn new_shaft_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    nbt.insert(DIRECTION.into(), Tag::Int(1));
    nbt.insert(WARMUP.into(), Tag::Int(rules::WARMUP_TICKS));
    nbt
}

/// `PoweredShaftBlockEntity.update`: the engine at `source` sets its power and direction. Writes
/// only what differs. True when it wrote.
fn update_shaft(nbt: &mut Compound, shaft: Pos, source: Pos, direction: i32, power: f32) -> bool {
    let key = sub(shaft, source);
    let now = read_driven(nbt);
    if now.engine == Some(key)
        && (now.power - power).abs() < 1e-5
        && now.direction == direction
        && util::has(nbt, ENGINE_TYPE)
    {
        return false;
    }
    nbt.insert(ENGINE_POS.into(), Tag::IntArray(key.to_vec()));
    util::set_float(nbt, ENGINE_POWER, power);
    nbt.insert(DIRECTION.into(), Tag::Int(direction));
    nbt.insert(ENGINE_TYPE.into(), Tag::String(ENGINE.into()));
    true
}

/// `PoweredShaftBlockEntity.remove`: the engine is gone.
fn clear_shaft(nbt: &mut Compound) {
    util::remove(nbt, ENGINE_POS);
    util::remove(nbt, ENGINE_POWER);
    util::remove(nbt, ENGINE_TYPE);
    nbt.insert(DIRECTION.into(), Tag::Int(0));
}

/// `PoweredShaftBlock.stillValid` for the shaft block at `pos`.
fn still_valid(host: &dyn Host, pos: Pos, axis: Axis) -> bool {
    rules::shaft_still_valid(axis, |offset| engine_at(host, add(pos, offset)))
}

/// The Powered Shaft's node: the engine's share as a generator (`getGeneratedSpeed`,
/// `calculateAddedStressCapacity`) on the shaft's axis.
fn shaft_node(axis: Axis, driven: &Driven) -> Node {
    Node {
        generated: rules::generated_speed(driven.direction, driven.power, ENGINE_CAPACITY),
        capacity: rules::added_capacity(driven.power, ENGINE_CAPACITY),
        ..Node::passive(create_kinetics::Kind::Shaft(axis))
    }
}

// --- the shaft's tick ------------------------------------------------------------------------------------

fn tick_shaft(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let Some(axis) = axis_of(&*host, pos) else {
        return;
    };
    // `PoweredShaftBlock.tick` (scheduled when the engine is removed): back to a Shaft.
    if !still_valid(&*host, pos, axis) {
        super::encased::switch_block(host, pos, SHAFT, &[("axis", axis.name())]);
        return;
    }

    let mut driven = read_driven(&event.entity.nbt);
    let mut wrote = false;
    // `tick`: the warm-up counts down; written only while it does.
    if driven.warmup > 0 {
        driven.warmup -= 1;
        event.entity.nbt.insert(WARMUP.into(), Tag::Int(driven.warmup));
        wrote = true;
    }
    // `remove(sourcePos)`: the engine that powered it has gone.
    if let Some(key) = driven.engine {
        let source = sub(pos, key);
        let pointing = engine_at(&*host, source).is_some_and(|c| add(source, scaled(c, 2)) == pos);
        if !pointing {
            clear_shaft(&mut event.entity.nbt);
            driven = read_driven(&event.entity.nbt);
            wrote = true;
        }
    }

    let node = shaft_node(axis, &driven);
    let Some((_, changed)) = kinetic::tick(host, pos, event.entity, node) else {
        return;
    };
    if changed || wrote {
        host.block_entity_sync(pos);
    }
}

// --- the engine's tick -------------------------------------------------------------------------------------

/// `PoweredShaftBlockEntity.canBePoweredBy` for the Powered Shaft entity at `shaft`.
fn can_power(entity: &ModBlockEntity, shaft: Pos, source: Pos) -> bool {
    let driven = read_driven(&entity.nbt);
    rules::can_be_powered_by(driven.warmup, driven.engine, source, shaft)
}

fn counter_clockwise(nbt: &Compound) -> bool {
    util::int(nbt, SCROLL) == 1
}

fn tick_engine(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let Some((connected, facing)) = engine_state(&*host, pos) else {
        return;
    };
    let dimension = host.dimension();
    let behind = sub(pos, connected.normal());
    let shaft_pos = add(pos, scaled(connected, 2));
    let now = util::game_time(host);

    // The tank behind it: `getTank`, and `canSurvive`.
    let Some(tank) = boiler::tank_at(&*host, behind) else {
        let chunk = |p: Pos| [p[0].div_euclid(16), p[2].div_euclid(16)];
        if host.chunk_is_loaded(dimension, chunk(behind)) && !boiler::is_tank(&host.block(behind)) {
            host.destroy_block(pos, None, false);
        }
        return;
    };
    boiler::register(dimension, tank.controller, pos, Attached::Engine, now);

    // `onPlace`: a Shaft in front of the engine becomes a Powered Shaft.
    let shaft_block = host.block(shaft_pos);
    if shaft_block == SHAFT
        && let Some(axis) = axis_of(&*host, shaft_pos)
        && rules::is_shaft_valid(connected, axis)
    {
        super::encased::switch_block(host, shaft_pos, POWERED_SHAFT, &[("axis", axis.name())]);
        return;
    }

    // `getShaft`: a Powered Shaft that can be powered by this engine.
    let shaft = (shaft_block == POWERED_SHAFT)
        .then(|| host.mod_block_entity(shaft_pos))
        .flatten()
        .filter(|entity| entity.id == POWERED_SHAFT && can_power(entity, shaft_pos, pos));
    let Some(shaft) = shaft else {
        return;
    };
    let driven = read_driven(&shaft.nbt);

    // `isValid`: only a plain Fluid Tank runs an engine, not a creative one.
    if tank.kind != super::fluids::FLUID_TANK {
        // The engine can't drive its shaft: the shaft that it powered stops.
        if driven.engine == Some(sub(shaft_pos, pos)) && driven.power != 0.0 {
            let (power, direction) = (0.0, 0);
            host.change_mod_block_entity(shaft_pos, &mut |e| {
                update_shaft(&mut e.nbt, shaft_pos, pos, direction, power);
            });
            host.block_entity_sync(shaft_pos);
        }
        return;
    }

    let Some(target_axis) = axis_of(&*host, shaft_pos) else {
        return;
    };
    // A floor or ceiling engine takes the direction its block faces (`facing.getAxis() == Y`).
    let facing_used = if connected.axis() == Axis::Y { facing } else { connected };
    let efficiency = tank.boiler.engine_efficiency(tank.size).clamp(0.0, 1.0);
    let mut conveyed = rules::conveyed_direction(
        efficiency,
        target_axis,
        facing_used,
        counter_clockwise(&event.entity.nbt),
    );

    // The shaft already turns with another source the other way: follow it.
    let shaft_speed = kinetic::speed(&shaft);
    let own = rules::generated_speed(driven.direction, driven.power, ENGINE_CAPACITY);
    let has_source = shaft_speed != 0.0
        && (own == 0.0
            || (own > 0.0) != (shaft_speed > 0.0)
            || (own.abs() - shaft_speed.abs()).abs() > 1e-3);
    if rules::must_flip(has_source, shaft_speed, conveyed) {
        let flipped = 1 - util::int(&event.entity.nbt, SCROLL);
        event.entity.nbt.insert(SCROLL.into(), Tag::Int(flipped));
        conveyed = -conveyed;
    }

    // `BoilerData.sound`, queued at each stroke of a working engine: a hiss on its side of the tank.
    if efficiency > 0.0 && rules_sound::engine_stroke(shaft_speed, now) {
        let dice = (util::random() % 1_000_000) as f32 / 1_000_000.0;
        let (volume, pitch) = rules_sound::boiler_hiss(tank.boiler.engines, dice);
        let at = util::centre(pos);
        host.play_sound("minecraft:block.candle.extinguish", at, 4, volume, pitch);
        util::create_sound(host, "steam", at, volume / 16.0, 0.8);
    }

    // `shaft.update(worldPosition, conveyedSpeedLevel, efficiency)`.
    // Compared on a copy first: reaching a block entity mutably counts as a change.
    let mut probe = shaft.nbt.clone();
    if update_shaft(&mut probe, shaft_pos, pos, conveyed, efficiency) {
        host.change_mod_block_entity(shaft_pos, &mut |e| {
            update_shaft(&mut e.nbt, shaft_pos, pos, conveyed, efficiency);
        });
        host.block_entity_sync(shaft_pos);
    }
}

// --- drawing ------------------------------------------------------------------------------------------------

/// `SteamEngineRenderer`: the piston, the linkage and the crank connector, moved by the angle of
/// the shaft in front of the engine.
fn draw_engine(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let pos = event.pos;
    let Some(connected) = connected(
        host.block_state_property(event.state, "face"),
        host.block_state_property(event.state, "facing"),
    ) else {
        return;
    };
    let shaft_pos = add(pos, scaled(connected, 2));
    let Some(axis) = axis_of(host, shaft_pos) else {
        return;
    };
    // `getShaft`: the shaft's entity, powered by this engine or by none yet.
    let Some(shaft) = host
        .mod_block_entity(shaft_pos)
        .filter(|entity| entity.id == POWERED_SHAFT && can_power(entity, shaft_pos, pos))
    else {
        return;
    };
    let offset = rules::position_offset(shaft_pos, axis) + rules::rotation_angle_offset(shaft_pos, axis);
    let draw = &mut *event.draw;
    let time = draw.game_time as f64 + f64::from(draw.partial);
    let angle = rules::shaft_angle(time, kinetic::speed(&shaft), offset);
    let Some(angle) = rules::target_angle(connected, axis, angle) else {
        return;
    };
    let roll = if rules::roll90(connected, axis) { -90.0 } else { 0.0 };
    let (piston, angle2) = rules::piston_pose(angle);
    let lift = piston + 20.0 / 16.0;

    // `transformed`: the model's block turned to the engine's facing.
    let orient = |draw: &mut crate::compat::BlockEntityDraw| {
        draw.rotate_about(Axis::Y.unit(), rules::horizontal_angle(connected), CENTRE);
        draw.rotate_about(Axis::X.unit(), rules::vertical_angle(connected) + 90.0, CENTRE);
        draw.rotate_about(Axis::Y.unit(), roll, CENTRE);
    };

    draw.push();
    orient(draw);
    draw.translate([0.0, lift, 0.0]);
    host.draw_model(draw, PISTON, 0xFF_FFFF);
    draw.pop();

    draw.push();
    orient(draw);
    draw.translate([0.0, 1.0, 0.0]);
    draw.translate([0.0, lift, 0.0]);
    draw.translate([0.0, 4.0 / 16.0, 8.0 / 16.0]);
    draw.rotate_about(Axis::X.unit(), angle2.to_degrees(), [0.0; 3]);
    draw.translate([0.0, -4.0 / 16.0, -8.0 / 16.0]);
    host.draw_model(draw, LINKAGE, 0xFF_FFFF);
    draw.pop();

    draw.push();
    orient(draw);
    draw.translate([0.0, 2.0, 0.0]);
    draw.rotate_about(
        Axis::X.unit(),
        (-(angle + std::f32::consts::FRAC_PI_2)).to_degrees(),
        CENTRE,
    );
    host.draw_model(draw, CONNECTOR, 0xFF_FFFF);
    draw.pop();
    // Everything above reads the game time and the partial tick: nothing is kept.
}

/// The shaft turns steadily with its speed (`SingleAxisRotatingVisual` of the powered shaft model).
fn draw_shaft(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let Some(axis) = host
        .block_state_property(event.state, "axis")
        .and_then(Axis::from_name)
    else {
        return;
    };
    let rpm = kinetic::speed(event.entity);
    let phase = rules::position_offset(event.pos, axis) + rules::rotation_angle_offset(event.pos, axis);
    let draw = &mut *event.draw;
    draw.spin(axis.unit(), CENTRE, rpm * 0.3, phase);
    host.draw_block(draw, POWERED_SHAFT, &[("axis", axis.name())]);
    draw.still();
    draw.keep();
}

// --- placement and the wrench ----------------------------------------------------------------------------------

/// Whether the state is one of a Fluid Tank, from what a level can say of it: only a tank has the
/// `window` shape (`FluidTankBlock.Shape`).
fn is_tank_state(level: &dyn LevelHost, state: BlockStateId) -> bool {
    level.state_with_property(state, "shape", "window").is_some()
}

/// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement` with `canSurvive`.
fn placement(
    level: &dyn LevelHost,
    event: &mut StateForPlacementEvent<'_>,
) -> Option<Option<BlockStateId>> {
    let context = event.context;
    let at = [context.pos.x, context.pos.y, context.pos.z];
    let found = rules::placement(
        context.yaw,
        context.pitch,
        util::dir(context.face),
        context.replace_clicked,
        |behind| is_tank_state(level, level.block_state(add(at, behind.normal()))),
    );
    let Some((face, facing)) = found else {
        // `canSurvive` fails everywhere: no placement.
        return Some(None);
    };
    let state = level.state_with_property(event.state, "face", face)?;
    level.state_with_property(state, "facing", facing.name()).map(Some)
}

/// `SteamEngineBlock.PlacementHelper`: clicking an engine with a Shaft puts a Shaft in front.
fn place_shaft(host: &mut dyn Host, engine: Pos, connected: Dir, face: Dir) -> Flow {
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    let _ = face;
    let shaft_pos = add(engine, scaled(connected, 2));
    let target = host.block(shaft_pos);
    // `BlockState.canBeReplaced`: air and fluids.
    if !matches!(
        target.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "minecraft:water" | "minecraft:lava"
    ) {
        return Flow::Cancel;
    }
    let axis = rules::helper_axis(player.yaw, player.pitch, connected);
    let creative = player.game_mode == util::CREATIVE;
    if !creative && host.take_items(SHAFT, 1) != 1 {
        return Flow::Cancel;
    }
    // The server places the Powered Shaft itself, as the helper does.
    if super::encased::switch_block(host, shaft_pos, POWERED_SHAFT, &[("axis", axis.name())]) {
        host.play_sound("minecraft:block.stone.place", util::centre(shaft_pos), 4, 1.0, 0.8);
    }
    Flow::Cancel
}

impl Hooks for Steam {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [PISTON, LINKAGE, CONNECTOR] {
            event.models.push(model.to_owned());
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        match event.block {
            ENGINE => {
                let mut nbt = Compound::new();
                nbt.insert(SCROLL.into(), Tag::Int(0));
                Some((ENGINE.to_owned(), nbt))
            }
            POWERED_SHAFT => Some((POWERED_SHAFT.to_owned(), new_shaft_entity())),
            _ => None,
        }
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ENGINE || event.block == POWERED_SHAFT {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        match event.entity.id.as_str() {
            ENGINE => tick_engine(host, event),
            POWERED_SHAFT => tick_shaft(host, event),
            _ => {}
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        match event.entity.id.as_str() {
            ENGINE => {
                let mut tag = Compound::new();
                tag.insert(
                    SCROLL.into(),
                    Tag::Int(util::int(&event.entity.nbt, SCROLL)),
                );
                Some(tag)
            }
            POWERED_SHAFT => {
                let nbt = &event.entity.nbt;
                let driven = read_driven(nbt);
                let mut tag = kinetic::update_tag(event.entity);
                // Always sent: a client keeps the old value of a key that is left out.
                tag.insert(DIRECTION.into(), Tag::Int(driven.direction));
                tag.insert(WARMUP.into(), Tag::Int(driven.warmup));
                tag.insert(
                    ENGINE_POS.into(),
                    Tag::IntArray(driven.engine.map(|p| p.to_vec()).unwrap_or_default()),
                );
                util::set_float(&mut tag, ENGINE_POWER, driven.power);
                Some(tag)
            }
            _ => None,
        }
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        match event.block {
            ENGINE => draw_engine(host, event),
            POWERED_SHAFT => draw_shaft(host, event),
            _ => {}
        }
    }

    /// `createRenderBoundingBox().inflate(2)`: the piston and the crank reach two blocks out.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block == ENGINE {
            for i in 0..3 {
                event.bounds[i] -= 2.0;
                event.bounds[i + 3] += 2.0;
            }
        }
    }

    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if &*event.context.item != ENGINE {
            return None;
        }
        placement(level, event)
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let item: &str = &event.stack.item;
        let block = host.block(pos);
        if block == ENGINE {
            let Some((connected, facing)) = engine_state(&*host, pos) else {
                return Flow::Continue;
            };
            if item == SHAFT {
                let face = Dir::from_normal(event.face).unwrap_or(Dir::Up);
                return place_shaft(host, pos, connected, face);
            }
            if item != util::WRENCH {
                return Flow::Continue;
            }
            let face = Dir::from_normal(event.face);
            return util::wrench(host, pos, |host| {
                // `IWrenchable.getRotatedBlockState`: from above or below, the facing turns
                // clockwise; a wall engine can't (its tank would no longer be behind it).
                if face.is_some_and(|f| f.axis() == Axis::Y) && connected.axis() == Axis::Y {
                    let next = match facing {
                        Dir::North => Dir::East,
                        Dir::East => Dir::South,
                        Dir::South => Dir::West,
                        _ => Dir::North,
                    };
                    host.set_block_property(pos, "facing", next.name());
                }
            });
        }
        if block == POWERED_SHAFT && item == util::WRENCH {
            let face = Dir::from_normal(event.face);
            return util::wrench(host, pos, |host| {
                // `RotatedPillarKineticBlock`: a quarter turn, if an engine still points at it.
                let (Some(face), Some(axis)) = (face, axis_of(&*host, pos)) else {
                    return;
                };
                let next = create_kinetics::rules::encased::rotate_axis(axis, face.axis());
                if next != axis && still_valid(&*host, pos, next) {
                    host.set_block_property(pos, "axis", next.name());
                }
            });
        }
        Flow::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_points_by_its_face_and_facing() {
        assert_eq!(connected(Some("floor"), Some("north")), Some(Dir::Up));
        assert_eq!(connected(Some("ceiling"), Some("east")), Some(Dir::Down));
        assert_eq!(connected(Some("wall"), Some("east")), Some(Dir::East));
        assert_eq!(connected(None, Some("east")), None);
        assert_eq!(connected(Some("wall"), None), None);
    }

    #[test]
    fn a_shaft_saves_its_engine_and_drops_it() {
        let mut nbt = new_shaft_entity();
        let driven = read_driven(&nbt);
        assert_eq!(
            driven,
            Driven {
                engine: None,
                power: 0.0,
                direction: 1,
                warmup: 3
            }
        );
        // A shaft at [0, 5, 0] powered by an engine at [0, 3, 0].
        assert!(update_shaft(&mut nbt, [0, 5, 0], [0, 3, 0], -1, 0.5));
        let driven = read_driven(&nbt);
        assert_eq!(driven.engine, Some([0, 2, 0]));
        assert_eq!((driven.power, driven.direction), (0.5, -1));
        // The same again changes nothing, so a block entity isn't marked changed.
        assert!(!update_shaft(&mut nbt, [0, 5, 0], [0, 3, 0], -1, 0.5));
        assert!(update_shaft(&mut nbt, [0, 5, 0], [0, 3, 0], 1, 0.5));
        clear_shaft(&mut nbt);
        let driven = read_driven(&nbt);
        assert_eq!((driven.engine, driven.power, driven.direction), (None, 0.0, 0));
    }

    #[test]
    fn a_powered_shaft_generates_what_its_engine_asks() {
        let driven = Driven {
            engine: Some([0, 2, 0]),
            power: 1.0,
            direction: -1,
            warmup: 0,
        };
        let node = shaft_node(Axis::X, &driven);
        assert_eq!(node.generated, -64.0);
        assert_eq!(node.capacity, 256.0);
        assert_eq!(node.impact, 0.0);
        let idle = shaft_node(Axis::X, &Driven { power: 0.0, ..driven });
        assert_eq!(idle.generated, 0.0);
    }

    #[test]
    fn the_engine_direction_setting_is_a_scroll_value() {
        let mut nbt = Compound::new();
        assert!(!counter_clockwise(&nbt));
        nbt.insert(SCROLL.into(), Tag::Int(1));
        assert!(counter_clockwise(&nbt));
    }
}
