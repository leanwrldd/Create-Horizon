"""The Packager and the Re-Packager: `create:packager`, `create:repackager`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.PACKAGER` and `REPACKAGER` (`BuilderTransformers.packager`:
`SharedProperties.softMetal`, `noOcclusion`, not a redstone conductor, `MapColor.TERRACOTTA_BLUE`,
`SoundType.NETHERITE_BLOCK`, `pickaxeOnly`; the Re-Packager's lang is "Re-Packager"), `PackagerBlock`
(`facing`, `powered`, `linked`; the Re-Packager has `facing` and `powered` only), `PackagerGenerator`
(the blockstate: `block`, `block_powered`, `block_linked`, `block_vertical*`, turned by `facing`), the
tray and hatch partial models (`AllPartialModels.PACKAGER_TRAY_REGULAR`, `PACKAGER_TRAY_DEFRAG`,
`PACKAGER_HATCH_OPEN`, `PACKAGER_HATCH_CLOSED`), `recipe/crafting/logistics/packager`,
`packager_from_conversion` and `repackager_from_conversion`. The behaviour is
`native/api/src/machines/packager.rs` (known differences at its top).
"""
from gen_content import FACING_STATE, block

TERRACOTTA_BLUE = 47  # MapColor.TERRACOTTA_BLUE
# `SharedProperties.softMetal`: a gold block; `SoundType.NETHERITE_BLOCK` on top.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True}
POWERED = {"powered": {"type": "boolean"}}

BLOCKS = {
    "packager": block({**FACING_STATE, **POWERED, "linked": {"type": "boolean"}},
                      {"facing": "north", "powered": "false", "linked": "false"},
                      SOFT_METAL, TERRACOTTA_BLUE, sounds="minecraft:netherite_block",
                      render_shape="MODEL"),
    "repackager": block({**FACING_STATE, **POWERED}, {"facing": "north", "powered": "false"},
                        SOFT_METAL, TERRACOTTA_BLUE, sounds="minecraft:netherite_block",
                        render_shape="MODEL"),
}
PICKAXE_ONLY = {"packager", "repackager"}

# What the block entity draws besides the block's model.
MODELS = ["block/packager/hatch_closed", "block/packager/hatch_open", "block/packager/tray",
          "block/repackager/tray"]

NAMES = {"block.create.packager": "Packager", "block.create.repackager": "Re-Packager"}
TAB = ["packager", "repackager"]

RECIPES = {
    "crafting/logistics/packager": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" C ", "CAC", "RCR"],
        "key": {"A": "create:cardboard_block", "C": "#c:ingots/iron", "R": "#c:dusts/redstone"},
        "result": {"id": "create:packager", "count": 1},
    },
    "crafting/logistics/packager_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:repackager"],
        "result": {"id": "create:packager", "count": 1},
    },
    "crafting/logistics/repackager_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:packager"],
        "result": {"id": "create:repackager", "count": 1},
    },
}
