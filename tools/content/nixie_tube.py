"""The Nixie Tubes (module `nixie_tube`): `create:nixie_tube` (orange) and `create:<colour>_nixie_tube`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ORANGE_NIXIE_TUBE` and `NIXIE_TUBES` (`softMetal` =
gold block's 3.0 / 6.0, `lightLevel(5)`, `mapColor(colour)`, `forceSolidOn`, `pickaxeOnly`;
the coloured ones have no item and drop the orange one), `NixieTubeBlock`
(`DoubleFaceAttachedBlock`: `double_face` and `facing`), `NixieTubeGenerator` (the blockstate file
in Create's jar turns `block/nixie_tube` by both) and the datagen recipe
`crafting/kinetics/nixie_tube` (two Electron Tubes make four). The behaviour is
`native/api/src/machines/nixie_tube.rs`.

Known differences from Create: no waterlogging, and no tube-shaped hit box (a declared shape can
follow one property only; the block is a full cube to the game's hit tests).
"""
from gen_content import block

SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:netherite_block", "light": 5}

# `DyeColor`: `(name, MapColor id)`, in ordinal order.
COLOURS = [("white", 8), ("orange", 15), ("magenta", 16), ("light_blue", 17), ("yellow", 18),
           ("lime", 19), ("pink", 20), ("gray", 21), ("light_gray", 22), ("cyan", 23),
           ("purple", 24), ("blue", 25), ("brown", 26), ("green", 27), ("red", 28), ("black", 29)]

STATES = {
    "double_face": {"type": "enum", "values": ["floor", "wall", "wall_reversed", "ceiling"]},
    "facing": {"type": "enum", "values": ["north", "south", "west", "east"]},
}
DEFAULT = {"double_face": "floor", "facing": "north"}

BLOCKS, NAMES, LOOT = {}, {}, {}
PICKAXE_ONLY = set()

for colour, map_color in COLOURS:
    name = "nixie_tube" if colour == "orange" else f"{colour}_nixie_tube"
    BLOCKS[name] = block(STATES, DEFAULT, SOFT_METAL, map_color, render_shape="MODEL")
    # The tube is see-through glass (`RenderType.translucent`), so it hides no neighbour's face.
    BLOCKS[name]["properties"]["occludes"] = False
    PICKAXE_ONLY.add(name)
    NAMES[f"block.create.{name}"] = ("Nixie Tube" if colour == "orange"
                                     else f"{colour.replace('_', ' ').title()} Nixie Tube")
    if colour != "orange":
        BLOCKS[name]["item"] = False
        # `dropOther(b, ORANGE_NIXIE_TUBE)`.
        LOOT[name] = [{"rolls": 1,
                       "entries": [{"type": "minecraft:item", "name": "create:nixie_tube"}],
                       "condition": {"type": "minecraft:survives_explosion"}}]

RECIPES = {
    # `crafting/kinetics/nixie_tube`: two Electron Tubes make four.
    "crafting/kinetics/nixie_tube": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:electron_tube", "create:electron_tube"],
        "result": {"id": "create:nixie_tube", "count": 4},
    },
}

TAB = ["nixie_tube"]
