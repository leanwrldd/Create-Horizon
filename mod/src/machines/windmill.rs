//! The Windmill Sail Frame and the Windmill Sails (`SailBlock`, `BlankSailBlockItem`,
//! `SailBlock.PlacementHelper`); the Windmill Bearing that turns them is in `bearing.rs`. Every
//! rule is in `create_kinetics::rules::windmill`; this reads the world, asks the rules and writes
//! what they say.
//!
//! A sail faces the way it is placed; a dye on a sail colours it (and, clicked again, the one
//! beside it, then the whole plane joined to it), shears take the canvas off, the Wrench turns it,
//! and a sail or frame used on a sail lays the next one of a row beside it.
//!
//! Known differences from Create, for the README:
//! - A dye isn't used up (as in Create) and the dye sound has a fixed pitch.
//! - The point of the click that picks the side of a dyed or placed neighbour is worked out from
//!   the player's look (a hook doesn't carry it).
//! - The Wrench's pickup and the sail's bounce when fallen on are Create's `IWrenchable` and
//!   `fallOn`; the bounce is not here.
//! - Dyed sails drop a white sail and have no item of their own, as Create's.

use create_kinetics::rules::contraption::{State, replaceable};
use create_kinetics::rules::funnel::nearest_looking;
use create_kinetics::rules::girder::hit_from_centre;
use create_kinetics::rules::windmill as rules;
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockStateId, Flow, Hooks, Host, LevelHost, StateForPlacementEvent, UseItemOnEvent,
};

use super::contraption::place_state;
use crate::util;

struct Sails;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Sails)
}

/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

fn state_at(host: &dyn Host, pos: Pos) -> Option<State> {
    let level = host.level(host.dimension())?;
    crate::compat::state_text(host, level, pos).map(|text| State::parse(&text))
}

fn is_sail_block(name: &str) -> bool {
    name == rules::FRAME || (name.starts_with("create:") && name.ends_with("_sail"))
}

fn eye(player: &crate::compat::NativeServerPlayer) -> [f64; 3] {
    let height = if player.sneaking { 1.27 } else { 1.62 };
    [player.position[0], player.position[1] + height, player.position[2]]
}

impl Hooks for Sails {
    /// `SailBlock.getStateForPlacement`: facing the way the player looks.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        let item: &str = &context.item;
        if item != rules::WHITE_SAIL && item != rules::FRAME {
            return None;
        }
        let facing = rules::placement_facing(nearest_looking(context.yaw, context.pitch)[0]);
        level
            .state_with_property(event.state, "facing", facing.name())
            .map(Some)
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        if !is_sail_block(&block) {
            return Flow::Continue;
        }
        let item: &str = &event.stack.item;
        let Some(clicked) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        if item == util::WRENCH {
            // `WrenchableDirectionalBlock.getRotatedBlockState`.
            return util::wrench(host, pos, |host| {
                let Some(facing) = util::facing(host, pos, "facing") else { return };
                let next = rules::wrenched_facing(facing, clicked);
                if next != facing {
                    host.set_block_property(pos, "facing", next.name());
                }
            });
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let hit = hit_from_centre(eye(&player), player.yaw, player.pitch, pos, clicked);
        let creative = player.game_mode == util::CREATIVE;

        // A sail or a frame on a sail: the next of the row (`PlacementHelper`).
        if (item == rules::WHITE_SAIL || item == rules::FRAME)
            && !player.sneaking
            && player.game_mode != util::SPECTATOR
            && player.game_mode != 2
        {
            let Some(facing) = util::facing(host, pos, "facing") else {
                return Flow::Continue;
            };
            let free = |at: Pos| {
                state_at(host, at).is_some_and(|s| s.is_air() || replaceable(&s))
            };
            if let Some((at, facing)) = rules::row_offset(facing, pos, hit, &free) {
                let placed = State::new(item).with("facing", facing.name());
                if (creative || host.take_items(item, 1) == 1) && place_state(host, at, &placed) {
                    host.play_sound("minecraft:block.scaffolding.place", util::centre(at), BLOCKS, 1.0, 0.8);
                }
            }
            return Flow::Cancel;
        }

        // Shears take the canvas off, a dye colours (`SailBlock.useItemOn`).
        let colour = if item == "minecraft:shears" {
            None
        } else if block != rules::FRAME {
            match rules::dye_colour(item) {
                Some(colour) => Some(colour),
                None => return Flow::Continue,
            }
        } else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR || player.game_mode == 2 {
            return Flow::Continue;
        }
        let sound = if colour.is_none() { "minecraft:entity.sheep.shear" } else { "minecraft:item.dye.use" };
        host.play_sound(sound, util::centre(pos), BLOCKS, 1.0, 1.0);
        let changes = {
            let host_ref: &dyn Host = &*host;
            rules::apply_dye(&|p| state_at(host_ref, p), pos, hit, colour)
        };
        for (at, state) in changes {
            place_state(host, at, &state);
        }
        Flow::Cancel
    }
}
