//! Vertical Gearbox (`VerticalGearboxItem`): an item that places a `create:gearbox` turned onto
//! its side. Its axis is the one the shaft ends beside it do not use, or, with none, the one
//! across the player's view. The rules are `create_kinetics::rules::vertical_gearbox`.
//!
//! Known differences from Create:
//! - A placement hook can't read neighbours, so the block is placed with the player's axis and
//!   lined up with a neighbouring shaft right after (`place_block`), as the encased shafts are.
//! - The item has Create's own name and model, and places the plain Gearbox block
//!   (`"place_block": "create:gearbox"`); the crafting recipes are Create's
//!   (`vertical_gearbox` and the two conversions).

use create_kinetics::rules::vertical_gearbox as rules;
use create_kinetics::{Axis, looking};
use horizon_client::mod_api::{
    BlockStateId, Hooks, Host, LevelHost, PlaceBlockEvent, StateForPlacementEvent,
};

use crate::machines::encased::neighbour_kind;

const ID: &str = "create:vertical_gearbox";

struct VerticalGearbox;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(VerticalGearbox)
}

impl Hooks for VerticalGearbox {
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if context.item != ID {
            return None;
        }
        let axis = rules::axis(None, looking(context.yaw, 0.0));
        level
            .state_with_property(event.state, "axis", axis.name())
            .map(Some)
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let preferred = rules::preferred_axis(|side| neighbour_kind(host, event.pos, side));
        let axis: Axis = rules::axis(preferred, looking(player.yaw, 0.0));
        if host.block_property(event.pos, "axis").as_deref() != Some(axis.name()) {
            host.set_block_property(event.pos, "axis", axis.name());
        }
    }
}
