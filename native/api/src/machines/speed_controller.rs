//! The Rotation Speed Controller in the game (`SpeedControllerBlock`, `SpeedControllerBlockEntity`,
//! `SpeedControllerRenderer`): a brass casing with a shaft through it that, under a large cogwheel,
//! drives that cogwheel at a speed set on the block (-256 to 256 RPM) for as long as its own shaft turns,
//! at whatever speed. What the output side costs comes out of the input side's generators.
//!
//! The rules are `create_kinetics::rules::speed_controller` and `Joint::SpeedController`
//! (`create_kinetics::solve` makes the cogwheel a generator); its header lists the known differences.
//! The ones that live here:
//!
//! - **The target speed, Create's scroll value box, is the Wrench's.** On a side of the block it steps
//!   the target forward on a face pointing the positive way (south, east) and back on a negative
//!   one (north, west) through `speed_controller::TARGETS` (0, then 1, 2, 4, 8, 16, 24, ... 256, and the
//!   same negative), and shows it on the action bar. On the top or the bottom it turns the block about
//!   the vertical axis as Create's Wrench does. Sneaking picks it up.
//! - The bracket under the cogwheel is polled by the server each tick and sent (Create's client checks on
//!   its lazy tick and when the block above changes).
//! - The block under a large cogwheel is turned across it when placed, in `place_block` rather than in
//!   `getStateForPlacement` (a placement hook can't read the block above by name).
//! - Create's `PlacementHelper` (right-clicking the controller with a large cogwheel places it above) and
//!   the ComputerCraft peripheral aren't here.

use create_kinetics::rules::speed_controller::{self as rules, DEFAULT_SPEED};
use create_kinetics::{Axis, Dir, Kind, Node, Pos, looking};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host,
    LevelHost, PlaceBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag, Text,
    UseItemOnEvent,
};

use super::encased;
use crate::{kinetic, util};

pub const ID: &str = "create:rotation_speed_controller";
/// `AllPartialModels.SPEED_CONTROLLER_BRACKET`.
const BRACKET_MODEL: &str = "create:block/rotation_speed_controller/bracket";
/// `ScrollValueBehaviour`'s key.
const SCROLL_VALUE: &str = "ScrollValue";
const BRACKET: &str = "Bracket";

const CENTRE: [f32; 3] = [0.5; 3];

struct Controller;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Controller)
}

fn target_of(nbt: &Compound) -> i32 {
    if util::has(nbt, SCROLL_VALUE) {
        util::int(nbt, SCROLL_VALUE)
    } else {
        DEFAULT_SPEED
    }
}

fn axis_at(host: &dyn Host, pos: Pos) -> Option<Axis> {
    host.block_property(pos, "axis").as_deref().and_then(Axis::from_name)
}

fn above(pos: Pos) -> Pos {
    [pos[0], pos[1] + 1, pos[2]]
}

/// `isCogwheelPresent`: a (bare) large cogwheel on a horizontal axis above.
fn bracket_wanted(host: &dyn Host, pos: Pos) -> bool {
    let up = above(pos);
    let block = host.block(up);
    let axis = host.block_property(up, "axis").as_deref().and_then(Axis::from_name);
    rules::has_bracket(&block, axis)
}

impl Hooks for Controller {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(BRACKET_MODEL.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            let mut nbt = kinetic::new_entity();
            nbt.insert(SCROLL_VALUE.into(), Tag::Int(DEFAULT_SPEED));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// The bracket stands in the block above.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block == ID {
            *event.bounds = [0.0, 0.0, 0.0, 1.0, 2.0, 1.0];
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some(axis) = axis_at(host, pos) else { return };
        // `CStress.setNoImpact`; the target is what the cogwheel above will be asked to turn at.
        let target = target_of(&event.entity.nbt);
        let node = Node::passive(Kind::speed_controller(axis, target));
        let Some((_, mut changed)) = kinetic::tick(host, pos, event.entity, node) else {
            return;
        };
        let bracket = bracket_wanted(host, pos);
        if util::boolean(&event.entity.nbt, BRACKET) != bracket {
            event.entity.nbt.insert(BRACKET.into(), Tag::Byte(i8::from(bracket)));
            changed = true;
        }
        if changed {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = kinetic::update_tag(event.entity);
        tag.insert(BRACKET.into(), Tag::Byte(i8::from(util::boolean(&event.entity.nbt, BRACKET))));
        tag.insert(SCROLL_VALUE.into(), Tag::Int(target_of(&event.entity.nbt)));
        Some(tag)
    }

    /// `SpeedControllerRenderer`: the shaft, and the bracket on a large cogwheel above.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(axis) = host
            .block_state_property(event.state, "axis")
            .and_then(Axis::from_name)
        else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        encased::draw_shaft(host, &mut *event.draw, axis, event.pos, rpm);
        if util::boolean(&event.entity.nbt, BRACKET) {
            let draw = &mut *event.draw;
            draw.push();
            draw.translate([0.0, 1.0, 0.0]);
            // `rotateCentered(alongX ? PI : PI / 2, UP)`.
            let turn = if axis == Axis::X { 180.0 } else { 90.0 };
            draw.rotate_about(Axis::Y.unit(), turn, CENTRE);
            host.draw_model(draw, BRACKET_MODEL, 0xFF_FFFF);
            draw.pop();
        }
        event.draw.keep();
    }

    /// `HorizontalAxisKineticBlock.getStateForPlacement`: across the way the player faces; a large
    /// cogwheel above or a shaft beside it is looked for after placing.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        let item: &str = &context.item;
        if item != ID {
            return None;
        }
        let axis = rules::look_axis(looking(context.yaw, 0.0));
        level.state_with_property(event.state, "axis", axis.name()).map(Some)
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        let pos = event.pos;
        let up = above(pos);
        let block = host.block(up);
        let cog_axis = host.block_property(up, "axis").as_deref().and_then(Axis::from_name);
        let preferred = rules::preferred_horizontal_axis(|side| {
            encased::neighbour_kind(host, pos, side).is_some_and(|kind| kind.has_shaft(side.opposite()))
        });
        let facing = looking(player.yaw, 0.0);
        let axis = rules::placement_axis(cog_axis.map(|axis| (block.as_str(), axis)), preferred, facing);
        if host.block_property(pos, "axis").as_deref() != Some(axis.name()) {
            host.set_block_property(pos, "axis", axis.name());
        }
    }

    /// The Wrench: the target speed on the sides, a quarter turn on the top and the bottom.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if &*event.stack.item != util::WRENCH || host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(face) = Dir::from_normal(event.face) else { return Flow::Continue };
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        util::wrench(host, pos, |host| {
            if face.axis() == Axis::Y {
                if let Some(axis) = axis_at(host, pos) {
                    let next = if axis == Axis::X { Axis::Z } else { Axis::X };
                    host.set_block_property(pos, "axis", next.name());
                }
                return;
            }
            let mut target = DEFAULT_SPEED;
            host.change_mod_block_entity(pos, &mut |entity| {
                target = rules::step_target(target_of(&entity.nbt), face.sign() > 0);
                if util::int(&entity.nbt, SCROLL_VALUE) != target || !util::has(&entity.nbt, SCROLL_VALUE) {
                    entity.nbt.insert(SCROLL_VALUE.into(), Tag::Int(target));
                }
            });
            host.block_entity_sync(pos);
            host.action_bar(player.uuid, Text::literal(format!("Targeted Speed in RPM: {target}")));
        })
    }
}
