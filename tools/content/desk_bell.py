"""The Desk Bell: `create:desk_bell`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.DESK_BELL` (`mapColor(SAND)`, default
`Properties.of()`, `directionalBlock` blockstate with `AssetLookup.forPowered`, `customItemModel`
of the block model), `DeskBellBlock` (`facing`, `powered`, `waterlogged`),
`AllShapes.DESK_BELL` (`shape(3, 0, 3, 13, 3, 13).add(4, 0, 4, 12, 9, 12).forDirectional(UP)`),
`recipe/crafting/logistics/desk_bell`, `AllSoundEvents.DESK_BELL_USE`
(`create:desk_bell`, in the jar's `sounds.json`). The bell is in the chunk mesh from Create's
blockstate file (the powered model has only the base); the block entity draws the plunger and the
bell while it rings (`AllPartialModels.DESK_BELL_PLUNGER`, `DESK_BELL_BELL`). The behaviour is
`native/api/src/machines/desk_bell.rs`; waterlogging is left out, as everywhere.
"""
from gen_content import FACING_STATE, block, facing_shapes

SAND = 2  # `MapColor.SAND`
# `Properties.of()`: no hardness, no tool, stone sounds.
PLAIN = {"hardness": 0.0, "explosion_resistance": 0.0, "requires_tool": False,
         "sounds": "minecraft:stone"}

_bell = block(
    {**FACING_STATE, "powered": {"type": "boolean"}},
    {"facing": "up", "powered": "false"}, PLAIN, SAND,
    facing_shapes([[3 / 16, 0, 3 / 16, 13 / 16, 3 / 16, 13 / 16],
                   [4 / 16, 0, 4 / 16, 12 / 16, 9 / 16, 12 / 16]]),
    render_shape="MODEL")

BLOCKS = {"desk_bell": _bell}

NAMES = {"block.create.desk_bell": "Desk Bell",
         "create.subtitle.desk_bell": "Reception bell dings"}

MODELS = ["block/desk_bell/plunger", "block/desk_bell/bell"]

RECIPES = {
    # `crafting/logistics/desk_bell`.
    "crafting/logistics/desk_bell": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:andesite_casing", "#c:plates/gold"],
        "result": {"id": "create:desk_bell"},
    },
}

TAB = ["desk_bell"]
