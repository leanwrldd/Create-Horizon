"""The Flywheel: `create:flywheel` (module `flywheel`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.FLYWHEEL` (`SharedProperties.softMetal`, `noOcclusion`,
`MapColor.TERRACOTTA_YELLOW`, `axeOrPickaxe`, `CStress.setNoImpact`), `FlywheelBlock` (a rotated
pillar: `axis`; `AllShapes.LARGE_GEAR`: the large gear's disc and a six-texel pole),
`recipe/crafting/kinetics/flywheel`. The wheel is Create's `.obj` (`block/flywheel/block`, no shaft),
which the block entity draws turning with the shaft; the behaviour is `native/api/src/machines/flywheel.rs`.
"""
from gen_content import AXIS_STATE, axis_shapes, block

TERRACOTTA_YELLOW = 40  # MapColor.TERRACOTTA_YELLOW
# `SharedProperties.softMetal`: a gold block.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:gold_block"}

# `AllShapes.LARGE_GEAR`: `LARGE_GEAR_SHAPE` (0, 6, 0, 16, 10, 16) and `SIX_VOXEL_POLE` (5, 0, 5, 11, 16, 11).
LARGE_GEAR = [[0, 6 / 16, 0, 1, 10 / 16, 1], [5 / 16, 0, 5 / 16, 11 / 16, 1, 11 / 16]]

BLOCKS = {
    # Drawn entirely by its block entity (`ENTITYBLOCK_ANIMATED`): the shaft and the wheel.
    "flywheel": block(AXIS_STATE, {"axis": "y"}, SOFT_METAL, TERRACOTTA_YELLOW, axis_shapes(LARGE_GEAR)),
}

# `FlywheelRenderer` draws the block's own model (the wheel without its shaft) round the axis.
MODELS = ["block/flywheel/block"]

NAMES = {"block.create.flywheel": "Flywheel"}

RECIPES = {
    # `crafting/kinetics/flywheel`.
    "crafting/kinetics/flywheel": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["CCC", "CAC", "CCC"],
        "key": {"A": "create:shaft", "C": "#c:ingots/brass"},
        "result": {"id": "create:flywheel", "count": 1},
    },
}

TAB = ["flywheel"]
