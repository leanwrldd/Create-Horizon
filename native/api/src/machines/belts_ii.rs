//! What the Mechanical Belt takes from items in hand (`BeltBlock.useItemOn` and `onWrenched`):
//! an Andesite or Brass Casing in hand puts a casing round the block clicked
//! (`BeltBlockEntity.setCasingType`), dye or a water bucket dyes or washes the whole belt
//! (`applyColor`), and the Wrench takes the casing off. The belt itself, the press over it, the
//! depot and the hand-off between them are `belt.rs`, `press.rs` and `depot.rs`.
//!
//! Known differences from Create (for the README):
//! - The casing doesn't make the block solid (no `RenderShape.MODEL`, the collision shape of the
//!   belt stays). An andesite casing is drawn as Create does, the brass casing model with its sprite
//!   shifted onto the andesite one (`BeltModel`), and the dye by the belt's scrolling sprite for
//!   that colour (`BeltRenderer`), both through Horizon's UV shift (`draw_terrain_model`).
//! - No `Covered` casing (a block above the belt that makes the casing a cover), and only dyes
//!   and a water bucket wash or dye; no other fluid container. The items are not used up, as in
//!   Create.
//! - The Wrench doesn't turn a pulley back into a middle block, and a Shaft doesn't make one.

use horizon_client::mod_api::{Flow, Hooks, Host, StandaloneModelsEvent, UseItemOnEvent};

use crate::belt::{self, CASING_MODELS};
use crate::util;

const ANDESITE_CASING: &str = "create:andesite_casing";
const BRASS_CASING: &str = "create:brass_casing";
const WATER_BUCKET: &str = "minecraft:water_bucket";
/// `CasingType` ordinals.
const ANDESITE: i32 = 1;
const BRASS: i32 = 2;
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

/// `DyeColor`'s names, by the dye item that gives them (`DyeColor.getColor(stack)`).
const DYES: [(&str, &str); 16] = [
    ("minecraft:white_dye", "WHITE"),
    ("minecraft:orange_dye", "ORANGE"),
    ("minecraft:magenta_dye", "MAGENTA"),
    ("minecraft:light_blue_dye", "LIGHT_BLUE"),
    ("minecraft:yellow_dye", "YELLOW"),
    ("minecraft:lime_dye", "LIME"),
    ("minecraft:pink_dye", "PINK"),
    ("minecraft:gray_dye", "GRAY"),
    ("minecraft:light_gray_dye", "LIGHT_GRAY"),
    ("minecraft:cyan_dye", "CYAN"),
    ("minecraft:purple_dye", "PURPLE"),
    ("minecraft:blue_dye", "BLUE"),
    ("minecraft:brown_dye", "BROWN"),
    ("minecraft:green_dye", "GREEN"),
    ("minecraft:red_dye", "RED"),
    ("minecraft:black_dye", "BLACK"),
];

struct BeltsII;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(BeltsII)
}

/// The `DyeColor` an item dyes with.
fn dye_of(item: &str) -> Option<&'static str> {
    DYES.iter().find(|(dye, _)| *dye == item).map(|(_, name)| *name)
}

impl Hooks for BeltsII {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in CASING_MODELS {
            if !event.models.iter().any(|m| m == model) {
                event.models.push(model.to_owned());
            }
        }
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let item = &*event.stack.item;
        let casing = match item {
            ANDESITE_CASING => Some(ANDESITE),
            BRASS_CASING => Some(BRASS),
            _ => None,
        };
        let wrench = item == util::WRENCH;
        if casing.is_none() && !wrench && dye_of(item).is_none() && item != WATER_BUCKET {
            return Flow::Continue;
        }
        if host.block(pos) != belt::ID {
            return Flow::Continue;
        }
        // `player.isShiftKeyDown() || !player.mayBuild()`: the item is used another way.
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if wrench {
            // `BeltBlock.onWrenched`: a casing comes off.
            if belt::casing(host, pos) == 0 {
                return Flow::Continue;
            }
            belt::set_casing(host, pos, 0);
            return Flow::Cancel;
        }
        if let Some(kind) = casing {
            belt::set_casing(host, pos, kind);
            // `getSoundType().getPlaceSound()` of a casing block (`SoundType.WOOD`).
            host.play_sound("minecraft:block.wood.place", util::centre(pos), BLOCKS, 1.0, 0.8);
            return Flow::Cancel;
        }
        // Dye, or water to wash it off (`DyeColor.getColor(bucket)` is none).
        let color = dye_of(item);
        if belt::apply_color(host, pos, color) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}
