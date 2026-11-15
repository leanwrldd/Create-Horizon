"""The Mechanical Bearing (module `bearing`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.MECHANICAL_BEARING` (`SharedProperties.stone`,
`MapColor.PODZOL`, axe or pickaxe, `noOcclusion`, `CStress.setImpact(4.0)`,
`BuilderTransformers.bearing("mechanical", "gearbox")`), `BearingBlock` (`facing`; waterlogging is
left out, as elsewhere), `AllPartialModels.BEARING_TOP` and `recipe/crafting/kinetics/
mechanical_bearing`.
"""

from gen_content import ANDESITE, FACING_STATE, PODZOL, block

BLOCKS = {
    # The base is in the chunk mesh (`create:block/mechanical_bearing`); the block entity draws
    # the shaft out of the back, the turning top and the structure.
    "mechanical_bearing": block(FACING_STATE, {"facing": "north"}, ANDESITE, PODZOL,
                                render_shape="MODEL"),
}

# What the block entity draws besides the block's model (`AllPartialModels.BEARING_TOP`).
MODELS = ["block/bearing/top"]

TAB = ["mechanical_bearing"]

NAMES = {"block.create.mechanical_bearing": "Mechanical Bearing"}

RECIPES = {
    # `crafting/kinetics/mechanical_bearing`, in Horizon's 26.3 form.
    "crafting/kinetics/mechanical_bearing": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "#minecraft:wooden_slabs", "C": "create:andesite_casing",
                "I": "create:shaft"},
        "result": {"id": "create:mechanical_bearing", "count": 1},
    },
}
