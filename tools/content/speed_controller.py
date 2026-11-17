"""The Rotation Speed Controller: `create:rotation_speed_controller` (module `speed_controller`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ROTATION_SPEED_CONTROLLER` (`SharedProperties.softMetal`,
`MapColor.TERRACOTTA_YELLOW`, `axeOrPickaxe`, `CStress.setNoImpact`), `SpeedControllerBlock`
(`HorizontalAxisKineticBlock`: the property `axis`, x or z; `AllShapes.SPEED_CONTROLLER`),
`AllPartialModels.SPEED_CONTROLLER_BRACKET`, `recipe/crafting/kinetics/rotation_speed_controller`.
The casing is in the chunk mesh; the block entity draws the shaft and, under a large cogwheel, the bracket.
The behaviour is `native/api/src/machines/speed_controller.rs`.
"""
from gen_content import block

TERRACOTTA_YELLOW = 40  # MapColor.TERRACOTTA_YELLOW
# `SharedProperties.softMetal`: a gold block.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:gold_block"}

# `AllShapes.SPEED_CONTROLLER = shape(0, 0, 0, 16, 4, 16).add(1, 1, 1, 15, 13, 15).add(0, 8, 0, 16, 14, 16)`.
SHAPE = [[0, 0, 0, 1, 4 / 16, 1], [1 / 16, 1 / 16, 1 / 16, 15 / 16, 13 / 16, 15 / 16],
         [0, 8 / 16, 0, 1, 14 / 16, 1]]
AXIS = {"axis": {"type": "enum", "values": ["x", "z"]}}

BLOCKS = {
    "rotation_speed_controller": block(
        AXIS, {"axis": "x"}, SOFT_METAL, TERRACOTTA_YELLOW,
        {"property": "axis", "values": {"x": SHAPE, "z": SHAPE}}, render_shape="MODEL"),
}

# `AllPartialModels.SPEED_CONTROLLER_BRACKET`.
MODELS = ["block/rotation_speed_controller/bracket"]

NAMES = {"block.create.rotation_speed_controller": "Rotation Speed Controller"}

RECIPES = {
    # `crafting/kinetics/rotation_speed_controller`.
    "crafting/kinetics/rotation_speed_controller": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C"],
        "key": {"B": "create:precision_mechanism", "C": "create:brass_casing"},
        "result": {"id": "create:rotation_speed_controller", "count": 1},
    },
}

TAB = ["rotation_speed_controller"]
