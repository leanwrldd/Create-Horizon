"""The Display Link (module `display_link`): `create:display_link`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.DISPLAY_LINK` (`SharedProperties.softMetal` = gold
block's 3.0 / 6.0, `MapColor.TERRACOTTA_BROWN`, `axeOrPickaxe`), `DisplayLinkBlock`
(`WrenchableDirectionalBlock`: `facing`; `powered`), `AllShapes.DATA_GATHERER`
(`shape(1, 0, 1, 15, 6, 15).add(3, 5, 3, 13, 9, 13).forDirectional()`) and the datagen recipe
`crafting/logistics/display_link`. The blockstate (`forPowered`: `block/display_link/block` or
`block_powered`, turned to the facing) and their composite models come from the player's jar; the
block entity draws the glowing bulb (`display_link/tube` and `glow`). The behaviour is
`native/api/src/machines/display_link.rs` (known differences at its top).
"""
from gen_content import FACINGS, block, facing_shapes

# `MapColor.TERRACOTTA_BROWN`.
TERRACOTTA_BROWN = 48
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": False,
              "sounds": "minecraft:netherite_block"}

# `DATA_GATHERER`, written for facing up and turned.
_BOXES = [[1 / 16, 0, 1 / 16, 15 / 16, 6 / 16, 15 / 16],
          [3 / 16, 5 / 16, 3 / 16, 13 / 16, 9 / 16, 13 / 16]]

BLOCKS = {
    "display_link": block(
        {"facing": {"type": "enum", "values": FACINGS}, "powered": {"type": "boolean"}},
        {"facing": "up", "powered": "false"},
        SOFT_METAL, TERRACOTTA_BROWN, facing_shapes(_BOXES), render_shape="MODEL"),
}
# `axeOrPickaxe()` (the default here).

NAMES = {"block.create.display_link": "Display Link"}

# The bulb's two partial models (`AllPartialModels.DISPLAY_LINK_TUBE` and `DISPLAY_LINK_GLOW`).
MODELS = ["create:block/display_link/tube", "create:block/display_link/glow"]

RECIPES = {
    # `crafting/logistics/display_link`: a Transmitter over a Brass Casing.
    "crafting/logistics/display_link": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["C", "S"],
        "key": {"C": "create:transmitter", "S": "create:brass_casing"},
        "result": {"id": "create:display_link"},
    },
}

TAB = ["display_link"]
