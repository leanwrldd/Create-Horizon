"""The Factory Gauge: `create:factory_gauge`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.FACTORY_GAUGE` (`SharedProperties.copperMetal`, `noOcclusion`, `forceSolidOn`,
`cutoutMipped`, `pickaxeOnly`, a `FactoryPanelBlockItem`, blockstate `horizontalFaceBlock` with the partial base model),
`FactoryPanelBlock` (`FaceAttachedHorizontalDirectionalBlock`: `face`, `facing`, `waterlogged` and `powered`; waterlogging is left out
as elsewhere), `AllShapes.FACTORY_PANEL_FALLBACK` (`shape(0, 0, 0, 16, 2, 16).forDirectional(UP)`, looked up by the connected direction;
Create builds the real shape from the active panels, `FactoryPanelBlockEntity.getShape`, and has no collision but for a null entity),
`recipe/crafting/logistics/factory_gauge` (`factory_gauge_clear` isn't declared: the item carries no frequency here),
`loot_table/blocks/factory_gauge`. The behaviour is `mod/src/machines/factory_gauge.rs`.
"""
from gen_content import block, facing_shapes

ORANGE = 15  # `Blocks.COPPER_BLOCK`'s `MapColor.COLOR_ORANGE`
# `SharedProperties.copperMetal` copies `Blocks.COPPER_BLOCK`.
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True, "sounds": "minecraft:copper_block"}
_HORIZONTAL = ["north", "south", "west", "east"]


def _shapes():
    """The slab's box for the connected direction: up on a floor, down on a ceiling, its `facing` on a wall."""
    by_direction = facing_shapes([[0, 0, 0, 1, 2 / 16, 1]])["values"]
    values = {}
    for facing in _HORIZONTAL:
        values[f"floor,{facing}"] = by_direction["up"]
        values[f"ceiling,{facing}"] = by_direction["down"]
        values[f"wall,{facing}"] = by_direction[facing]
    return {"properties": ["face", "facing"], "values": values, "default": values["wall,north"]}


_gauge = block(
    {"face": {"type": "enum", "values": ["floor", "wall", "ceiling"]},
     "facing": {"type": "enum", "values": _HORIZONTAL},
     "powered": {"type": "boolean"}},
    {"face": "wall", "facing": "north", "powered": "false"},
    COPPER, ORANGE, _shapes(), render_shape="MODEL")
# `getCollisionShape`: empty unless the collision context has no entity.
_gauge["collision_shapes"] = {"default": []}

BLOCKS = {"factory_gauge": _gauge}
PICKAXE_ONLY = {"factory_gauge"}

NAMES = {"block.create.factory_gauge": "Factory Gauge"}
TAB = ["factory_gauge"]

RECIPES = {
    "crafting/logistics/factory_gauge": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:stock_link", "create:precision_mechanism"],
        "result": {"id": "create:factory_gauge", "count": 2},
    },
}
