"""The Display Board (module `flap_display`): `create:display_board`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.DISPLAY_BOARD` (`SharedProperties.softMetal` = gold
block's 3.0 / 6.0, `MapColor.COLOR_GRAY`, `pickaxeOnly`, `CStress.setNoImpact`),
`FlapDisplayBlock` (`HorizontalKineticBlock`: `facing`; `up` and `down` join boards),
`AllShapes.FLAP_DISPLAY` (`shape(0, 0, 3, 16, 16, 13).forHorizontal(SOUTH)`) and the datagen
recipe `crafting/kinetics/display_board` (two from two Andesite Alloy and an Electron Tube). The
blockstate and model (`block/display_board/block`) come from the player's jar; the block entity
draws the cogwheel and the text. The behaviour is `native/api/src/machines/flap_display.rs`
(known differences at its top).
"""
from gen_content import block

# `MapColor.COLOR_GRAY`.
GRAY = 21
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:netherite_block"}

# 3..13 texels deep: the same either way round for the two south-north facings and the two others.
_SHAPE = {"property": "facing", "values": {
    "north": [[0, 0, 3 / 16, 1, 1, 13 / 16]], "south": [[0, 0, 3 / 16, 1, 1, 13 / 16]],
    "east": [[3 / 16, 0, 0, 13 / 16, 1, 1]], "west": [[3 / 16, 0, 0, 13 / 16, 1, 1]]}}

BLOCKS = {
    "display_board": block(
        {"facing": {"type": "enum", "values": ["north", "south", "west", "east"]},
         "up": {"type": "boolean"}, "down": {"type": "boolean"}},
        {"facing": "north", "up": "false", "down": "false"},
        SOFT_METAL, GRAY, _SHAPE, render_shape="MODEL"),
}
PICKAXE_ONLY = {"display_board"}

NAMES = {"block.create.display_board": "Display Board"}

MODELS = ["create:block/cogwheel_shaftless"]

RECIPES = {
    # `crafting/kinetics/display_board`: `PAP`, two boards.
    "crafting/kinetics/display_board": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["PAP"],
        "key": {"A": "create:electron_tube", "P": "create:andesite_alloy"},
        "result": {"id": "create:display_board", "count": 2},
    },
}

TAB = ["display_board"]
