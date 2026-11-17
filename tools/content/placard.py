"""The Placard: `create:placard`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.PLACARD` (`SharedProperties.copperMetal`,
`forceSolidOn`, `pickaxeOnly`, tag `SAFE_NBT`, `horizontalFaceBlock` blockstate with
`AssetLookup.standardModel`, `simpleItem`), `PlacardBlock` (`face`, `facing`, `powered`,
`waterlogged`), `AllShapes.PLACARD` (`shape(2, 0, 2, 14, 3, 14).forDirectional(UP)`),
`recipe/crafting/kinetics/placard`, `loot_table/blocks/placard`. The plate is in the chunk mesh
from Create's blockstate file; the block entity draws the held item. The behaviour is
`native/api/src/machines/placard.rs`; waterlogging is left out, as everywhere.
"""
from gen_content import block, facing_shapes

ORANGE = 15  # `Blocks.COPPER_BLOCK`'s `MapColor.COLOR_ORANGE`
# `SharedProperties.copperMetal` copies `Blocks.COPPER_BLOCK`.
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}

_BOXES = facing_shapes([[2 / 16, 0, 2 / 16, 14 / 16, 3 / 16, 14 / 16]])["values"]


def _shapes():
    """The plate's box for the direction the plate faces: up on a floor, down on a ceiling, its
    `facing` on a wall (`FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`)."""
    values = {}
    for facing in ("north", "south", "west", "east"):
        values[f"floor,{facing}"] = _BOXES["up"]
        values[f"ceiling,{facing}"] = _BOXES["down"]
        values[f"wall,{facing}"] = _BOXES[facing]
    return {"properties": ["face", "facing"], "values": values, "default": values["wall,north"]}


_placard = block(
    {"face": {"type": "enum", "values": ["floor", "wall", "ceiling"]},
     "facing": {"type": "enum", "values": ["north", "south", "west", "east"]},
     "powered": {"type": "boolean"}},
    {"face": "wall", "facing": "north", "powered": "false"},
    COPPER, ORANGE, _shapes(), render_shape="MODEL")

BLOCKS = {"placard": _placard}
PICKAXE_ONLY = {"placard"}

NAMES = {"block.create.placard": "Placard",
         "create.subtitle.confirm": "Affirmative ding",
         "create.subtitle.deny": "Declining boop"}

RECIPES = {
    # `crafting/kinetics/placard`.
    "crafting/kinetics/placard": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["minecraft:item_frame", "#c:plates/brass"],
        "result": {"id": "create:placard"},
    },
}

TAGS = {"create:block/safe_nbt": ["create:placard"]}

TAB = ["placard"]
