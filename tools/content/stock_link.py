"""The Stock Link: `create:stock_link`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.STOCK_LINK` (`SharedProperties.softMetal`, `MapColor.TERRACOTTA_BLUE`,
`SoundType.NETHERITE_BLOCK`, `pickaxeOnly`, a `LogisticallyLinkedBlockItem`), `PackagerLinkBlock`
(`FaceAttachedHorizontalDirectionalBlock`: `face`, `facing`, and `powered`; waterlogging left out as elsewhere),
`AllShapes.STOCK_LINK` (`shape(1, 0, 1, 15, 5, 15).forDirectional()`, looked up by the connected direction),
`PackagerLinkGenerator` (the blockstate: `block_vertical[_powered]` and `block_horizontal[_powered]`, in Create's jar),
`data/create/recipe/crafting/logistics/stock_link`. The `_clear` recipe (it resets a tuned item) isn't declared: the item
carries no frequency here. The behaviour is `mod/src/machines/stock_link.rs` and `logistics.rs`.
"""
from gen_content import block, facing_shapes

TERRACOTTA_BLUE = 47  # MapColor.TERRACOTTA_BLUE
# `SharedProperties.softMetal`.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True}
_HORIZONTAL = ["north", "south", "west", "east"]


def _shapes():
    """`forDirectional` boxes, picked by `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`: the ceiling is DOWN,
    the floor UP, a wall the way the block faces."""
    by_direction = facing_shapes([[1 / 16, 0, 1 / 16, 15 / 16, 5 / 16, 15 / 16]])["values"]
    values = {}
    for facing in _HORIZONTAL:
        values[f"floor,{facing}"] = by_direction["up"]
        values[f"ceiling,{facing}"] = by_direction["down"]
        values[f"wall,{facing}"] = by_direction[facing]
    return {"properties": ["face", "facing"], "values": values, "default": values["wall,north"]}


BLOCKS = {
    "stock_link": block(
        {"face": {"type": "enum", "values": ["floor", "wall", "ceiling"]},
         "facing": {"type": "enum", "values": _HORIZONTAL},
         "powered": {"type": "boolean"}},
        {"face": "wall", "facing": "north", "powered": "false"},
        SOFT_METAL, TERRACOTTA_BLUE, _shapes(), sounds="minecraft:netherite_block", render_shape="MODEL"),
}
PICKAXE_ONLY = {"stock_link"}

NAMES = {"block.create.stock_link": "Stock Link"}
TAB = ["stock_link"]

RECIPES = {
    "crafting/logistics/stock_link": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["C", "B"],
        "key": {"B": "create:item_vault", "C": "create:transmitter"},
        "result": {"id": "create:stock_link", "count": 1},
    },
}
