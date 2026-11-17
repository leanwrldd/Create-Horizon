"""The Sticker: `create:sticker` (module `sticker`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.STICKER` (`SharedProperties.stone`, `pickaxeOnly`, `noOcclusion`,
cutout, `directionalBlock(forPowered)`), `StickerBlock` (`facing`, `powered`, `extended`: a
`WrenchableDirectionalBlock`), `AllPartialModels.STICKER_HEAD`, `recipe/crafting/kinetics/sticker`.
The behaviour is `native/api/src/machines/sticker.rs`; what the glue does to a structure is in the contraption
rules.
"""

from gen_content import ANDESITE, FACING_STATE, block

STONE = 11  # MapColor.STONE

BLOCKS = {
    "sticker": block({**FACING_STATE, "powered": {"type": "boolean"}, "extended": {"type": "boolean"}},
                     {"facing": "north", "powered": "false", "extended": "false"}, ANDESITE, STONE,
                     render_shape="MODEL"),
}
# `pickaxeOnly()`.
PICKAXE_ONLY = {"sticker"}

# What the block entity draws besides the block's model.
MODELS = ["block/sticker/head"]

TAB = ["sticker"]

NAMES = {"block.create.sticker": "Sticker"}

RECIPES = {
    # `recipe/crafting/kinetics/sticker`, in Horizon's 26.3 form.
    "crafting/kinetics/sticker": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["ISI", "CRC"],
        "key": {"I": "create:andesite_alloy", "C": "#c:cobblestones", "R": "#c:dusts/redstone",
                "S": "#c:slimeballs"},
        "result": {"id": "create:sticker", "count": 1},
    },
}
