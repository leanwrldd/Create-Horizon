"""The Weighted Ejector (module `weighted_ejector`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.WEIGHTED_EJECTOR` (`SharedProperties.stone`, `noOcclusion`,
`MapColor.COLOR_GRAY`, `axeOrPickaxe`, `CStress.setImpact(2.0)`, `EjectorItem`), `EjectorBlock`
(`facing`, `AllShapes.CASING_13PX` facing up; waterlogging is left out, as elsewhere),
`AllPartialModels.EJECTOR_TOP`, `recipe/crafting/kinetics/weighted_ejector`.
"""

from gen_content import ANDESITE, GRAY, HORIZONTAL_FACING_STATE, block

# The casing is in the chunk mesh (`block/weighted_ejector/block`); the block entity draws the shaft,
# the lid, the held stack and what is in the air.
BLOCKS = {
    "weighted_ejector": block(HORIZONTAL_FACING_STATE, {"facing": "north"}, ANDESITE, GRAY,
                              {"default": [[0, 0, 0, 1, 13 / 16, 1]]}, render_shape="MODEL"),
}

NAMES = {"block.create.weighted_ejector": "Weighted Ejector"}

MODELS = ["create:block/weighted_ejector/top"]

RECIPES = {
    # `crafting/kinetics/weighted_ejector`.
    "weighted_ejector": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["A", "D", "I"],
        "key": {"A": "#c:plates/gold", "D": "create:depot", "I": "create:cogwheel"},
        "result": {"id": "create:weighted_ejector"},
    },
}

TAB = ["weighted_ejector"]
