//! The Mechanical Roller as a block: its wheel and frame, its placement and the sharing of its settings along a line
//! of rollers (`RollerBlock`, `RollerBlockEntity`, `RollerRenderer`). What it does on a moving structure is
//! `roller_actor.rs`; the rules are `create_kinetics::rules::roller`. The filter and the mode are value boxes
//! (`value_box.rs`, the rows in `rules::value_box_blocks`), saved as `Filter` and `ScrollValue`.
//!
//! Known differences from Create:
//! - `RollerBlockItem.place` (a roller put on a block with a full top face goes one up) isn't ported: the host's
//!   placement event has no way to move the block.
//! - `RollerBlock.PlacementHelper` (the pole helper that extends a line of rollers on a click) isn't ported.
//! - `AttachedActorBlock`'s waterlogging is left out, as everywhere.
//! - `RollerBlockEntity.isValidMaterial` asks vanilla blocks from the baked list in `roller_facts.rs`; another
//!   mod's block is taken as valid.
//! - The stationary wheel stands still (`getAnimatedSpeed` is a Ponder detail).

use create_kinetics::rules::roller as rules;
use create_kinetics::rules::value_box::horizontal_angle;
use create_kinetics::{Axis, Dir, Pos};
use crate::compat::TextExt;
use crate::compat::{
    BlockEntityDraw, BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockStateId,
    Compound, Hooks, Host, LevelHost, NativeServerPlayer, PlaceBlockEvent, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, Text,
};

use super::chute_inv;
use super::value_box::SCROLL_VALUE;
use crate::{int, util};

pub const ROLLER: &str = "create:mechanical_roller";
/// `AllPartialModels.ROLLER_WHEEL`, a `.obj` mesh.
pub const WHEEL: &str = "create:block/mechanical_roller/wheel";
/// `AllPartialModels.ROLLER_FRAME`.
pub const FRAME: &str = "create:block/mechanical_roller/frame";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

struct Rollers;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Rollers)
}

// --- the drawing ---------------------------------------------------------------------------------------

/// `RollerRenderer`'s wheel: down a quarter block, out in front of the block by 17 pixels, turned to the facing,
/// then about the block's west axis at `rate` degrees a tick (`HarvesterRenderer.transform` with a zero pivot), and
/// set down on the frame (`translate(0, -.5, .5).rotateYDegrees(90)`).
pub(super) fn draw_wheel(host: &dyn Host, draw: &mut BlockEntityDraw, facing: Dir, rate: f32) {
    draw.push();
    draw.translate([0.0, -0.25, 0.0]);
    let n = facing.normal();
    draw.translate([n[0] as f32 * 17.0 / 16.0, n[1] as f32 * 17.0 / 16.0, n[2] as f32 * 17.0 / 16.0]);
    crate::draw::face_south(draw, facing);
    if rate != 0.0 {
        // About `Direction.WEST`.
        draw.spin([-1.0, 0.0, 0.0], [0.0; 3], rate, 0.0);
    }
    draw.translate([0.0, -0.5, 0.5]);
    draw.rotate_about([0.0, 1.0, 0.0], 90.0, [0.0; 3]);
    host.draw_model(draw, WHEEL, 0xFF_FFFF);
    draw.still();
    draw.pop();
}

/// `RollerRenderer`'s frame: the block's turn by `horizontalAngle(facing) + 180` about its centre.
pub(super) fn draw_frame(host: &dyn Host, draw: &mut BlockEntityDraw, facing: Dir) {
    draw.push();
    draw.rotate_about([0.0, 1.0, 0.0], (horizontal_angle(facing) + 180.0) as f32, [0.5; 3]);
    host.draw_model(draw, FRAME, 0xFF_FFFF);
    draw.pop();
}

// --- settings shared along a line ------------------------------------------------------------------------------

/// Whether `at` holds a roller in the same state as the one at `pos` (`level.getBlockState(pos) != blockState`;
/// the waterlogging isn't kept here) with a block entity.
fn same_roller(host: &dyn Host, pos: Pos, at: Pos) -> bool {
    host.block(at) == ROLLER
        && util::facing(host, at, "facing") == util::facing(host, pos, "facing")
        && host.mod_block_entity(at).is_some()
}

/// `acceptSharedValues`: the mode and the filter set without passing them on.
fn accept(host: &mut dyn Host, at: Pos, mode: i32, filter: &chute_inv::Filter) {
    host.change_mod_block_entity(at, &mut |entity| {
        entity.nbt.insert(SCROLL_VALUE.into(), Tag::Int(mode));
        chute_inv::write_filter(&mut entity.nbt, filter);
    });
    host.block_entity_sync(at);
}

/// `shareValuesToAdjacent`: the rollers in a line across this one's facing, up to 99 blocks each way and up to the
/// first gap, take its mode and filter.
pub fn share_to_adjacent(host: &mut dyn Host, pos: Pos) {
    let Some(entity) = host.mod_block_entity(pos) else { return };
    let Some(facing) = util::facing(&*host, pos, "facing") else { return };
    let across = facing.clockwise_about(Axis::Y).normal();
    let mode = int(&entity.nbt, SCROLL_VALUE);
    let filter = chute_inv::filter_of(&entity.nbt);
    for side in rules::SIDES {
        for i in 1..rules::SHARE_REACH {
            let at = rules::line_cell(pos, across, side, i);
            if !same_roller(&*host, pos, at) {
                break;
            }
            accept(host, at, mode, &filter);
        }
    }
}

/// `searchForSharedValues`, after a roller is placed: it takes the settings of the roller next to it, and passes them
/// on along the line.
fn search_for_shared_values(host: &mut dyn Host, pos: Pos) {
    let Some(facing) = util::facing(&*host, pos, "facing") else { return };
    let across = facing.clockwise_about(Axis::Y).normal();
    for side in rules::SIDES {
        let at = rules::line_cell(pos, across, side, 1);
        if !same_roller(&*host, pos, at) {
            continue;
        }
        let Some(other) = host.mod_block_entity(at) else { continue };
        accept(host, pos, int(&other.nbt, SCROLL_VALUE), &chute_inv::filter_of(&other.nbt));
        share_to_adjacent(host, pos);
        break;
    }
}

// --- the filter ----------------------------------------------------------------------------------------------

/// `RollerBlockEntity.isValidMaterial` for an item held: a block item the roller can pave with.
pub fn accepts(host: &dyn Host, item: &str) -> bool {
    rules::is_valid_material(item, host.default_state(item).is_some())
}

/// `FilteringBehaviour.onShortInteract` when the filter is refused: "Cannot use this item here." and the deny sound.
pub fn deny(host: &mut dyn Host, player: &NativeServerPlayer) {
    let line = Text::translate_with_fallback("create.logistics.filter.invalid_item", "Cannot use this item here.", &[]);
    host.action_bar(player.uuid, line);
    util::create_sound(host, "deny", player.position, 1.0, 1.0);
}

// --- the hooks ---------------------------------------------------------------------------------------------------

impl Hooks for Rollers {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(WHEEL.to_owned());
        event.models.push(FRAME.to_owned());
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ROLLER).then(|| (ROLLER.to_owned(), Compound::new()))
    }

    /// `RollerRenderer.renderSafe`: the wheel at rest and the frame.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ROLLER {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        // `RollerRenderer`: the wheel turns as `HarvesterRenderer.transform` says; a scene sets the speed.
        let angle = crate::ponder::animated_rate(&event.entity.nbt);
        draw_wheel(host, &mut *event.draw, facing, angle.unwrap_or(0.0));
        draw_frame(host, &mut *event.draw, facing);
        if angle.is_none() {
            event.draw.keep();
        }
    }

    /// `RollerBlockEntity.createRenderBoundingBox`: the block grown by one.
    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block == ROLLER {
            *event.bounds = [-1.0, -1.0, -1.0, 2.0, 2.0, 2.0];
        }
    }

    /// `RollerBlock.getStateForPlacement`: facing away from the player, whatever face was clicked.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ROLLER {
            return None;
        }
        let facing = create_kinetics::looking(context.yaw, 0.0).opposite();
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// `RollerBlock.setPlacedBy`: `searchForSharedValues`.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item == ROLLER && host.block(event.pos) == ROLLER {
            search_for_shared_values(host, event.pos);
        }
    }
}
