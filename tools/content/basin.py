"""The Basin: `create:basin` and the `create:mixing` / `create:compacting` recipes it serves
(the recipe tables are in `basin_recipes.py`).

Create: `AllBlocks.BASIN` (`SharedProperties.stone`, `MapColor.COLOR_GRAY`, `SoundType.NETHERITE_BLOCK`,
`pickaxeOnly`), `BasinBlock` (`FACING_HOPPER`), `AllShapes.BASIN_*`, `BasinGenerator`,
`recipe/crafting/kinetics/basin`.
"""

from gen_content import ANDESITE, GRAY, block

# `BlockStateProperties.FACING_HOPPER`: down and the four horizontal sides.
FACING_HOPPER = {"facing": {"type": "enum", "values": ["down", "north", "south", "west", "east"]}}


def _bowl(top, floor):
    """A bowl as boxes in blocks: four 2 pixel walls from y 2 to `top` pixels, a floor up to
    `floor` pixels (`erase` leaves the interior open from there) and the 12 pixel base."""
    boxes = [
        [0, 2, 0, 2, top, 16], [14, 2, 0, 16, top, 16],
        [2, 2, 0, 14, top, 2], [2, 2, 14, 14, top, 16],
        [2, 0, 2, 14, 2, 14],
    ]
    if floor > 2:
        boxes.append([2, 2, 2, 14, floor, 14])
    return [[v / 16 for v in box] for box in boxes]


# `AllShapes.BASIN_BLOCK_SHAPE = shape(0, 2, 0, 16, 16, 16).erase(2, 2, 2, 14, 16, 14).add(2, 0, 2, 14, 2, 14)`.
BASIN_BLOCK_SHAPE = _bowl(16, 2)
# `BASIN_COLLISION_SHAPE = shape(0, 2, 0, 16, 13, 16).erase(2, 5, 2, 14, 16, 14).add(2, 0, 2, 14, 2, 14)`:
# what dropped items meet (`BasinBlock.getCollisionShape` for an `ItemEntity`).
BASIN_COLLISION_SHAPE = _bowl(13, 5)
# `BASIN_RAYTRACE_SHAPE = shape(0, 2, 0, 16, 16, 16).add(2, 0, 2, 14, 2, 14)`: what a click lands on.
BASIN_RAYTRACE_SHAPE = [[0, 2 / 16, 0, 1, 1, 1], [2 / 16, 0, 2 / 16, 14 / 16, 2 / 16, 14 / 16]]

_basin = block(FACING_HOPPER, {"facing": "down"}, ANDESITE, GRAY,
               {"default": BASIN_BLOCK_SHAPE}, sounds="minecraft:netherite_block",
               render_shape="MODEL")
# Players meet the walls; items fall into the bowl; clicks land on the solid shape.
_basin["collision_context_shapes"] = {"item": {"default": BASIN_COLLISION_SHAPE}}
_basin["interaction_shapes"] = {"default": BASIN_RAYTRACE_SHAPE}

BLOCKS = {"basin": _basin}
PICKAXE_ONLY = {"basin"}
NAMES = {"block.create.basin": "Basin"}
TAB = ["basin"]

RECIPES = {
    # `recipe/crafting/kinetics/basin`.
    "basin": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["A A", "AAA"],
        "key": {"A": "create:andesite_alloy"},
        "result": {"id": "create:basin"},
    },
}
