"""Schematics (module `schematic`): `create:schematic_and_quill`, `create:empty_schematic`,
`create:schematic`, `create:schematic_table` and `create:schematicannon`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.EMPTY_SCHEMATIC`, `SCHEMATIC_AND_QUILL` and `SCHEMATIC`
(`stacksTo(1)`), `AllBlocks.SCHEMATICANNON` (`Blocks.DISPENSER`'s properties, `MapColor.COLOR_GRAY`,
`pickaxeOnly`, `AllShapes.SCHEMATICANNON_SHAPE`, the base model `block/schematicannon/block`, its
connector and pipe drawn by `SchematicannonRenderer`) and `SCHEMATIC_TABLE` (`Blocks.LECTERN`'s
properties, `MapColor.PODZOL`, `axeOrPickaxe`, `HorizontalDirectionalBlock`, `AllShapes.SCHEMATICS_TABLE`
(`forDirectional(SOUTH)`) and `TABLE_POLE_SHAPE` as its collision), the recipes
`crafting/schematics/*`. The schematicannon's Create loot table also copies its settings
(`SCHEMATICANNON_OPTIONS`) onto the dropped item; here it drops plain. The behaviour is
`native/api/src/machines/schematic*.rs`.
"""
from gen_content import FACINGS, GRAY, PODZOL, block

_S = 1 / 16

# `AllShapes.SCHEMATICANNON_SHAPE`: shape(1, 0, 1, 15, 8, 15).add(0.5, 8, 0.5, 15.5, 11, 15.5).
_CANNON = [[1 * _S, 0, 1 * _S, 15 * _S, 8 * _S, 15 * _S],
           [0.5 * _S, 8 * _S, 0.5 * _S, 15.5 * _S, 11 * _S, 15.5 * _S]]

# `AllShapes.SCHEMATICS_TABLE`: shape(4, 0, 4, 12, 12, 12).add(0, 11, 2, 16, 14, 14), written for
# south and turned about the block's centre for the other facings (`forDirectional(SOUTH)`).
_TABLE_SOUTH = [[4, 0, 4, 12, 12, 12], [0, 11, 2, 16, 14, 14]]


def _turn(box, facing):
    """A box in texels written for south, turned to `facing` about the block's centre."""
    x0, y0, z0, x1, y1, z1 = box
    if facing == "south":
        return [x0, y0, z0, x1, y1, z1]
    if facing == "north":
        return [16 - x1, y0, 16 - z1, 16 - x0, y1, 16 - z0]
    if facing == "west":  # a quarter turn clockwise seen from above: south -> west
        return [16 - z1, y0, x0, 16 - z0, y1, x1]
    return [z0, y0, 16 - x1, z1, y1, 16 - x0]  # east, counterclockwise


def _table_shapes():
    return {"property": "facing",
            "values": {f: [[v * _S for v in _turn(b, f)] for b in _TABLE_SOUTH]
                       for f in ("north", "south", "west", "east")}}


# `AllShapes.TABLE_POLE_SHAPE`, the table's collision: shape(4, 0, 4, 12, 2, 12).add(5, 2, 5, 11, 14, 11).
_POLE = [[4 * _S, 0, 4 * _S, 12 * _S, 2 * _S, 12 * _S], [5 * _S, 2 * _S, 5 * _S, 11 * _S, 14 * _S, 11 * _S]]

# `Blocks.DISPENSER` and `Blocks.LECTERN` as vanilla's prepared data has them.
_DISPENSER = {"hardness": 3.5, "explosion_resistance": 3.5, "requires_tool": True,
              "sounds": "minecraft:stone"}
_LECTERN = {"hardness": 2.5, "explosion_resistance": 2.5, "requires_tool": False,
            "sounds": "minecraft:oak_planks"}

# Horizon opens a mod block's container menu only when the template's class opens a menu on use (a glass
# template never does), so both blocks use the crafting table, which has no states of its own; the
# menu itself is the hopper-shaped one of their `ModContainer`.
_OPENS_A_MENU = "minecraft:crafting_table"

BLOCKS = {
    "schematicannon": block(None, None, _DISPENSER, GRAY,
                            {"default": _CANNON}, render_shape="MODEL", template=_OPENS_A_MENU),
    "schematic_table": block(
        {"facing": {"type": "enum", "values": ["north", "south", "west", "east"]}},
        {"facing": "north"}, _LECTERN, PODZOL, _table_shapes(), render_shape="MODEL",
        template=_OPENS_A_MENU),
}
BLOCKS["schematic_table"]["collision_shapes"] = {"default": _POLE}
# `forceSolidOn` on the table, and neither block hides its neighbours' faces.
for _name in BLOCKS:
    BLOCKS[_name]["properties"]["occludes"] = False

ITEMS = {
    "empty_schematic": {"template": "minecraft:iron_ingot", "max_stack_size": 1},
    "schematic_and_quill": {"template": "minecraft:iron_ingot", "max_stack_size": 1},
    "schematic": {"template": "minecraft:iron_ingot", "max_stack_size": 1},
}

NAMES = {
    "block.create.schematicannon": "Schematicannon",
    "block.create.schematic_table": "Schematic Table",
    "item.create.empty_schematic": "Empty Schematic",
    "item.create.schematic_and_quill": "Schematic And Quill",
    "item.create.schematic": "Schematic",
}

PICKAXE_ONLY = {"schematicannon"}

# The partial models `SchematicannonRenderer` draws (`AllPartialModels.SCHEMATICANNON_CONNECTOR`
# and `_PIPE`).
MODELS = ["block/schematicannon/connector", "block/schematicannon/pipe"]

TAGS = {
    # `c:feathers` (the schematic and quill's recipe); vanilla has no `c:` tags.
    "c:item/feathers": ["minecraft:feather"],
}

RECIPES = {
    # `crafting/schematics/empty_schematic`.
    "crafting/schematics/empty_schematic": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["minecraft:paper", "#c:dyes/light_blue"],
        "result": {"id": "create:empty_schematic", "count": 1},
    },
    # `crafting/schematics/schematic_and_quill`.
    "crafting/schematics/schematic_and_quill": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:empty_schematic", "#c:feathers"],
        "result": {"id": "create:schematic_and_quill", "count": 1},
    },
    # `crafting/schematics/schematic_table`.
    "crafting/schematics/schematic_table": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["WWW", " S ", " S "],
        "key": {"W": "#minecraft:wooden_slabs", "S": "minecraft:smooth_stone"},
        "result": {"id": "create:schematic_table", "count": 1},
    },
    # `crafting/schematics/schematicannon`.
    "crafting/schematics/schematicannon": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" I ", "LIL", "SDS"],
        "key": {"D": "minecraft:dispenser", "I": "minecraft:iron_block", "L": "#minecraft:logs",
                "S": "minecraft:smooth_stone"},
        "result": {"id": "create:schematicannon", "count": 1},
    },
}

# `AllCreativeModeTabs`: the schematics sit with the base blocks (the written Schematic is left out).
TAB = ["schematicannon", "schematic_table", "empty_schematic", "schematic_and_quill"]
