//! The Large Water Wheel and the blocks it fills (`LargeWaterWheelBlock`,
//! `LargeWaterWheelBlockEntity`, `LargeWaterWheelBlockItem`, `WaterWheelStructuralBlock`,
//! `WaterWheelBlockEntity`, `WaterWheelRenderer`): a wheel three blocks across with its centre block
//! on the axle, eight `create:water_wheel_structure` blocks round it, and a generator that takes
//! the current of the twelve blocks around its rim. The rules and numbers are
//! `create_kinetics::rules::large_water_wheel`; this is the glue.
//!
//! Known differences from Create, for the README:
//! - The wheel looks at the water every 4 ticks (Create: every 60, and on a neighbour update), the
//!   same as the small wheel here does every tick. Its structure blocks and the `extension` state
//!   are checked every 10 ticks, and a structure block checks its wheel every 20, where Create does it
//!   on updates: a broken wheel's blocks can stand up to a second.
//! - No materials: a plank in the hand doesn't change the wheel's look (`applyMaterialIfValid`,
//!   `Material`), it is always spruce, as the small wheel here is. No waterlogging. No advancement.
//! - Placing it needs all eight blocks round it free (`canBeReplaced`, from the baked table of
//!   `contraption_facts`); the click that fails says so on the action bar and places nothing.
//!   Create's second try one block further out along the clicked face
//!   (`LargeWaterWheelBlockItem.place`) is not made, and the outline of the space is not shown.
//! - The wheel takes the axis the player looks along; it doesn't line up with a shaft beside it
//!   (`getPreferredAxis`).
//! - Breaking a structure block as a player breaks the wheel (`playerWillDestroy`); one removed
//!   another way (an explosion, a piston) leaves the wheel standing until its own check puts it back.
//! - The wheel isn't drawn lit from inside its own blocks and its render bounds are a fixed box.

use create_kinetics::rules::contraption_facts::REPLACEABLE;
use create_kinetics::rules::large_water_wheel as rules;
use create_kinetics::{Axis, Dir, Kind, Node, looking};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, DestroyBlockEvent, Flow,
    Hooks, Host, LevelHost, PlaceBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag, Text,
    UseItemOnEvent,
};

use crate::{kinetic, util};

pub const WHEEL: &str = "create:large_water_wheel";
pub const STRUCTURE: &str = "create:water_wheel_structure";
const FLOW_SCORE: &str = "FlowScore";
const BLOCK_MODEL: &str = "create:block/large_water_wheel/block";
const EXTENSION_MODEL: &str = "create:block/large_water_wheel/block_extension";
const CENTRE: [f32; 3] = [0.5; 3];
/// Ticks between two looks at the water.
const FLOW_EVERY: i64 = 4;
/// Ticks between two checks of the structure blocks round the wheel.
const STRUCTURE_EVERY: i64 = 10;
/// Ticks between two checks of a structure block for its wheel.
const MASTER_EVERY: i64 = 20;

struct Wheel;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Wheel)
}

type Pos = [i32; 3];

fn add(a: Pos, b: Pos) -> Pos {
    util::add(a, b)
}

fn replaceable(block: &str) -> bool {
    REPLACEABLE.contains(&block)
}

fn axis_of(host: &dyn Host, pos: Pos) -> Option<Axis> {
    host.block_property(pos, "axis").as_deref().and_then(Axis::from_name)
}

/// The shaft ends along `axis`.
fn ends(axis: Axis) -> Vec<Dir> {
    Dir::ALL.into_iter().filter(|d| d.axis() == axis).collect()
}

/// `salt`: spreads blocks' periodic work over the ticks.
fn salt(pos: Pos) -> i64 {
    i64::from(pos[0]) + 3 * i64::from(pos[1]) + 7 * i64::from(pos[2])
}

/// `WaterWheelBlockEntity.getFlowVectorAtPosition`: the current at `at`, or straight up or down in
/// a bubble column.
fn flow_at(host: &dyn Host, dimension: &str, at: Pos) -> [f64; 3] {
    if host.block(at) == "minecraft:bubble_column" {
        let down = host.block_property(at, "drag").as_deref() == Some("true");
        return [0.0, if down { -1.0 } else { 1.0 }, 0.0];
    }
    host.fluid_flow(dimension, at).unwrap_or([0.0; 3])
}

/// `WaterWheelStructuralBlock.stillValid`, from the structure block at `pos` through any chain of
/// structure blocks: the large wheel it belongs to, if it is still there.
pub(super) fn master_of(host: &dyn Host, pos: Pos) -> Option<Pos> {
    let mut at = pos;
    // The chain is at most two blocks long; the bound only stops a bad loop.
    for _ in 0..4 {
        if host.block(at) != STRUCTURE {
            return None;
        }
        let facing = util::facing(host, at, "facing")?;
        let target = rules::master_step(at, facing);
        let name = host.block(target);
        if name == STRUCTURE {
            at = target;
            continue;
        }
        if name != WHEEL {
            return None;
        }
        let axis = axis_of(host, target)?;
        // The wheel must turn about an axis other than the one the last structure block faces along
        // (`targetedState.getValue(AXIS) != direction.getAxis()`).
        return rules::faces_wheel(facing, axis).then_some(target);
    }
    None
}

/// `LargeWaterWheelBlock.tick`: puts the eight structure blocks round the wheel at `pos`, or breaks
/// the wheel when one of them is taken by something that can't be replaced.
fn fill_structure(host: &mut dyn Host, pos: Pos, axis: Axis) {
    let dimension = host.dimension();
    for (offset, facing) in rules::structure(axis) {
        let at = add(pos, offset);
        let name = host.block(at);
        if name == STRUCTURE && util::facing(host, at, "facing") == Some(facing) {
            continue;
        }
        if name != STRUCTURE && !replaceable(&name) {
            // `pLevel.destroyBlock(pPos, false)`.
            host.destroy_block(pos, None, false);
            return;
        }
        if let Some(state) = host.default_state(STRUCTURE) {
            host.set_block(dimension, at, state, util::UPDATE_ALL);
            host.set_block_property(at, "facing", facing.name());
        }
    }
}

/// `LargeWaterWheelBlock.updateShape`: `extension` while another large wheel is on the negative side
/// of the axis.
fn update_extension(host: &mut dyn Host, pos: Pos, axis: Axis) {
    let negative = Dir::ALL.into_iter().find(|d| d.axis() == axis && d.sign() < 0);
    let Some(negative) = negative else {
        return;
    };
    let extension = host.block(add(pos, negative.normal())) == WHEEL;
    let now = host.block_property(pos, "extension").as_deref() == Some("true");
    if extension != now {
        host.set_block_property(pos, "extension", if extension { "true" } else { "false" });
    }
}

impl Hooks for Wheel {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.extend([BLOCK_MODEL, EXTENSION_MODEL].map(str::to_owned));
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        match event.block {
            WHEEL => {
                let mut nbt = kinetic::new_entity();
                nbt.insert(FLOW_SCORE.into(), Tag::Int(0));
                Some((WHEEL.to_owned(), nbt))
            }
            STRUCTURE => Some((STRUCTURE.to_owned(), Compound::new())),
            _ => None,
        }
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == WHEEL || event.block == STRUCTURE {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        let pos = event.pos;
        if event.entity.id == STRUCTURE {
            // `WaterWheelStructuralBlock.tick`: gone once its wheel is.
            if (util::game_time(host) + salt(pos)).rem_euclid(MASTER_EVERY) == 0
                && host.chunk_is_loaded(host.dimension(), [pos[0] >> 4, pos[2] >> 4])
                && master_of(&*host, pos).is_none()
                && let Some(air) = host.default_state("minecraft:air")
            {
                let dimension = host.dimension();
                host.set_block(dimension, pos, air, util::UPDATE_ALL);
            }
            return;
        }
        if event.entity.id != WHEEL {
            return;
        }
        let Some(axis) = axis_of(&*host, pos) else {
            return;
        };
        let now = util::game_time(host) + salt(pos);
        let mut score = util::int(&event.entity.nbt, FLOW_SCORE);
        let mut score_changed = false;
        if now.rem_euclid(FLOW_EVERY) == 0 {
            // `determineAndApplyFlowScore`.
            let dimension = host.dimension();
            let new = {
                let host: &dyn Host = &*host;
                rules::score(axis, |offset| flow_at(host, dimension, add(pos, offset)))
            };
            if new != score {
                score = new;
                score_changed = true;
                event.entity.nbt.insert(FLOW_SCORE.into(), Tag::Int(score));
            }
        }

        let node = Node {
            generated: rules::speed(score),
            capacity: rules::CAPACITY,
            ..Node::passive(Kind::shafts(&ends(axis)))
        };
        let Some((_rpm, speed_changed)) = kinetic::tick(host, pos, event.entity, node) else {
            return; // The network jammed and the block broke.
        };
        if speed_changed || score_changed {
            host.block_entity_sync(pos);
        }
        if now.rem_euclid(STRUCTURE_EVERY) == 0 {
            update_extension(host, pos, axis);
            fill_structure(host, pos, axis);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        (event.entity.id == WHEEL).then(|| {
            let mut tag = kinetic::update_tag(event.entity);
            tag.insert(FLOW_SCORE.into(), Tag::Int(util::int(&event.entity.nbt, FLOW_SCORE)));
            tag
        })
    }

    /// `AABB(worldPosition).inflate(getSize())`.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block == WHEEL {
            let size = rules::SIZE as f32;
            for i in 0..3 {
                event.bounds[i] = event.bounds[i].min(-size);
                event.bounds[i + 3] = event.bounds[i + 3].max(1.0 + size);
            }
        }
    }

    /// `WaterWheelRenderer`: the wheel turns with its shaft, the model with the extension when
    /// there is another wheel on the negative side.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != WHEEL {
            return;
        }
        let Some(axis) = host.block_state_property(event.state, "axis").and_then(Axis::from_name) else {
            return;
        };
        let extension = host.block_state_property(event.state, "extension").as_deref() == Some("true");
        let rpm = kinetic::speed(event.entity);
        let draw = &mut *event.draw;
        draw.spin(axis.unit(), CENTRE, rpm * 0.3, 0.0);
        draw.push();
        // The model is made standing on the Y axis; a wheel on X or Z lies down as the small one does.
        match axis {
            Axis::X => draw.rotate_about(Axis::Z.unit(), -90.0, CENTRE),
            Axis::Z => draw.rotate_about(Axis::X.unit(), 90.0, CENTRE),
            Axis::Y => {}
        }
        host.draw_model(draw, if extension { EXTENSION_MODEL } else { BLOCK_MODEL }, 0xFF_FFFF);
        draw.pop();
        draw.still();
        draw.keep();
    }

    /// `RotatedPillarKineticBlock.getStateForPlacement` without a shaft to follow: the wheel takes the
    /// axis the player looks along.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if &*event.context.item != WHEEL {
            return None;
        }
        let axis = looking(event.context.yaw, event.context.pitch).axis();
        level.state_with_property(event.state, "axis", axis.name()).map(Some)
    }

    /// `LargeWaterWheelBlock.getStateForPlacement`: nothing is placed unless the eight blocks round
    /// the wheel are free. The click is taken here, before the block goes down.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        if &*event.stack.item == util::WRENCH {
            return wrench(host, pos);
        }
        if &*event.stack.item != WHEEL {
            return Flow::Continue;
        }
        let (Some(player), Some(clicked)) = (host.acting_player(), clicked) else {
            return Flow::Continue;
        };
        // Where the block would go: the clicked block if it can be replaced, else beside it.
        let target = if replaceable(&host.block(pos)) { pos } else { add(pos, clicked.normal()) };
        let axis = looking(player.yaw, player.pitch).axis();
        let free = rules::footprint(axis).iter().all(|offset| {
            let at = add(target, *offset);
            replaceable(&host.block(at))
        });
        if !free {
            host.action_bar(player.uuid, Text::literal("Clear Blocks for Placement".to_owned()));
            return Flow::Cancel;
        }
        Flow::Continue
    }

    /// `onPlace`: the structure goes up and the wheel takes its extension at once.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != WHEEL {
            return;
        }
        let pos = event.pos;
        if let Some(axis) = axis_of(&*host, pos) {
            update_extension(host, pos, axis);
            fill_structure(host, pos, axis);
        }
    }

    /// `WaterWheelStructuralBlock.playerWillDestroy`: a player breaking a structure block breaks the
    /// wheel (with its drop in survival).
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        if host.block(event.pos) != STRUCTURE {
            return Flow::Continue;
        }
        let Some(master) = master_of(&*host, event.pos) else {
            return Flow::Continue;
        };
        if let Some(player) = host.acting_player() {
            host.player_break_block(player.uuid, master, player.game_mode != util::CREATIVE);
        }
        Flow::Cancel
    }
}

/// The Wrench: sneaking on any block of the wheel picks the whole wheel up; a plain click does
/// nothing (`onWrenched` is `PASS`).
fn wrench(host: &mut dyn Host, pos: Pos) -> Flow {
    let block = host.block(pos);
    if block == WHEEL {
        return util::wrench(host, pos, |_| {});
    }
    if block == STRUCTURE {
        return match master_of(&*host, pos) {
            Some(master) => util::wrench(host, master, |_| {}),
            None => Flow::Cancel,
        };
    }
    Flow::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheels_shaft_ends_are_the_two_faces_on_its_axis() {
        for axis in Axis::ALL {
            let ends = ends(axis);
            assert_eq!(ends.len(), 2);
            assert!(ends.iter().all(|d| d.axis() == axis));
        }
    }

    #[test]
    fn only_air_water_and_plants_are_free_to_build_in() {
        for free in ["minecraft:air", "minecraft:water", "minecraft:short_grass", "minecraft:lava"] {
            assert!(replaceable(free));
        }
        for solid in ["minecraft:stone", "create:water_wheel_structure", "create:shaft"] {
            assert!(!replaceable(solid));
        }
    }
}
