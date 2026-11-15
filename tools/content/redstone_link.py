"""The Redstone Link: `create:redstone_link`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.REDSTONE_LINK` (`SharedProperties.wooden`,
`MapColor.TERRACOTTA_BROWN`, `forceSolidOn`, `axeOrPickaxe`, tags `BRITTLE` and `SAFE_NBT`),
`RedstoneLinkBlock` (`facing` from `WrenchableDirectionalBlock`, `powered`, `receiver`),
`AllShapes.REDSTONE_BRIDGE`, `RedstoneLinkGenerator` (the blockstate file in Create's jar picks
`block/redstone_link/<transmitter|receiver>[_vertical][_powered]`),
`data/create/recipe/crafting/logistics/redstone_link.json`. The block sits in the chunk mesh from
Create's blockstate file; the block entity draws the two frequency items. The behaviour is
`native/api/src/machines/redstone_link.rs` (known differences at its top).
"""
from gen_content import FACINGS, WOODEN, block

# `MapColor.TERRACOTTA_BROWN`.
TERRACOTTA_BROWN = 48


def _s(v):
    return v / 16


# `AllShapes.REDSTONE_BRIDGE = shape(VERTICAL_TABLET_SHAPE).forDirectional(SOUTH)
# .withVerticalShapes(LOGISTICAL_CONTROLLER.get(UP))`: `VERTICAL_TABLET_SHAPE = cuboid(3, 1, -1, 13,
# 15, 3)` on the four sides and `SQUARE_TABLET_SHAPE = cuboid(2, 2, -1, 14, 14, 3)` up and down,
# in texels, cut to the block's own cell (see `rules::redstone_link::shape`).
SHAPES = {
    "property": "facing",
    "values": {
        "south": [[_s(3), _s(1), 0, _s(13), _s(15), _s(3)]],
        "north": [[_s(3), _s(1), _s(13), _s(13), _s(15), 1]],
        "east": [[0, _s(1), _s(3), _s(3), _s(15), _s(13)]],
        "west": [[_s(13), _s(1), _s(3), 1, _s(15), _s(13)]],
        "up": [[_s(2), 0, _s(2), _s(14), _s(3), _s(14)]],
        "down": [[_s(2), _s(13), _s(2), _s(14), 1, _s(14)]],
    },
}

BLOCKS = {
    "redstone_link": block(
        {"facing": {"type": "enum", "values": FACINGS},
         "powered": {"type": "boolean"}, "receiver": {"type": "boolean"}},
        {"facing": "north", "powered": "false", "receiver": "false"},
        WOODEN, TERRACOTTA_BROWN, SHAPES, render_shape="MODEL"),
}

NAMES = {"block.create.redstone_link": "Redstone Link"}

RECIPES = {
    # `crafting/logistics/redstone_link`: two from a Transmitter over an Andesite Casing.
    "crafting/logistics/redstone_link": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["C", "S"],
        "key": {"C": "create:transmitter", "S": "create:andesite_casing"},
        "result": {"id": "create:redstone_link", "count": 2},
    },
}

# `AllBlockTags.BRITTLE` and `SAFE_NBT`, as `AllBlocks.REDSTONE_LINK` adds them.
TAGS = {
    "create:block/brittle": ["create:redstone_link"],
    "create:block/safe_nbt": ["create:redstone_link"],
}

TAB = ["redstone_link"]
