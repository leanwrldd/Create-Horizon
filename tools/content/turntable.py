"""The Turntable (module `turntable`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.TURNTABLE` (`SharedProperties.wooden`,
`MapColor.PODZOL`, `axeOrPickaxe`, `CStress.setImpact(4.0)`, `simpleItem`), `AllShapes.TURNTABLE_SHAPE`
(shape(1, 4, 1, 15, 8, 15).add(5, 0, 5, 11, 4, 11)), `recipe/crafting/kinetics/turntable`.
"""

from gen_content import PODZOL, WOODEN, block

# The whole block turns with its shaft (`KineticBlockEntityRenderer`), so the block entity draws it
# and the model stays out of the chunk mesh.
BLOCKS = {
    "turntable": block({}, {}, WOODEN, PODZOL,
                       {"default": [[1 / 16, 4 / 16, 1 / 16, 15 / 16, 8 / 16, 15 / 16],
                                    [5 / 16, 0, 5 / 16, 11 / 16, 4 / 16, 11 / 16]]}),
}

NAMES = {"block.create.turntable": "Turntable"}

RECIPES = {
    # `crafting/kinetics/turntable`.
    "turntable": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["P", "S"],
        "key": {"P": "#minecraft:wooden_slabs", "S": "create:shaft"},
        "result": {"id": "create:turntable"},
    },
}

TAB = ["turntable"]
