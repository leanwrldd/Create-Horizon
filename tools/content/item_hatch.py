"""The Item Hatch: `create:item_hatch`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ITEM_HATCH` (`SharedProperties.softMetal`, `MapColor.TERRACOTTA_BLUE`,
`SoundType.NETHERITE_BLOCK`, `pickaxeOnly`, `cutoutMipped`, blockstate `horizontalBlock` with the partial models
`block_open` / `block_closed`), `ItemHatchBlock` (`facing`, `open`; waterlogging left out as elsewhere), `AllShapes.ITEM_HATCH`
(`shape(1, 0, 0, 15, 16, 2).add(2, 2, 0, 14, 13, 3.8).add(2, 4, 0, 14, 11, 5.8).add(2, 6, 0, 14, 9, 7.8).forHorizontal(SOUTH)`,
looked up by the opposite of `facing`), `data/create/recipe/crafting/logistics/item_hatch`. The behaviour is
`mod/src/machines/item_hatch.rs`.
"""
from gen_content import HORIZONTAL_FACING_STATE, block

TERRACOTTA_BLUE = 47  # MapColor.TERRACOTTA_BLUE
# `SharedProperties.softMetal`.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True}

# Written for a plate on the north side (`forHorizontal(SOUTH)` looked up by the opposite of `facing`: the plate is on the side the
# hatch faces), in texels.
_PLATE = [[1, 0, 0, 15, 16, 2], [2, 2, 0, 14, 13, 3.8], [2, 4, 0, 14, 11, 5.8], [2, 6, 0, 14, 9, 7.8]]


def _turned(boxes, facing):
    """The boxes turned so the plate is on the side `facing` points to, in blocks."""
    out = []
    for x0, y0, z0, x1, y1, z1 in ([v / 16 for v in box] for box in boxes):
        if facing == "north":
            out.append([x0, y0, z0, x1, y1, z1])
        elif facing == "south":
            out.append([1 - x1, y0, 1 - z1, 1 - x0, y1, 1 - z0])
        elif facing == "east":
            out.append([1 - z1, y0, x0, 1 - z0, y1, x1])
        else:
            out.append([z0, y0, 1 - x1, z1, y1, 1 - x0])
    return [[round(v, 6) for v in box] for box in out]


SHAPES = {"property": "facing",
          "values": {f: _turned(_PLATE, f) for f in ("north", "south", "east", "west")}}

BLOCKS = {
    "item_hatch": block(
        {**HORIZONTAL_FACING_STATE, "open": {"type": "boolean"}}, {"facing": "north", "open": "false"},
        SOFT_METAL, TERRACOTTA_BLUE, SHAPES, sounds="minecraft:netherite_block", render_shape="MODEL"),
}
PICKAXE_ONLY = {"item_hatch"}

NAMES = {"block.create.item_hatch": "Item Hatch"}
TAB = ["item_hatch"]

RECIPES = {
    "crafting/logistics/item_hatch": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:andesite_alloy", "minecraft:iron_trapdoor"],
        "result": {"id": "create:item_hatch", "count": 1},
    },
}
