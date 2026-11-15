"""The Mechanical Pump: `create:mechanical_pump`.

Create: `AllBlocks.MECHANICAL_PUMP` (`SharedProperties.copperMetal`, `MapColor.STONE`,
`pickaxeOnly`, `CStress.setImpact(4.0)`), `PumpBlock` (a `DirectionalKineticBlock`),
`AllShapes.PUMP = shape(2, 0, 2, 14, 16, 14).forDirectional(UP)`, `PumpRenderer`,
`recipe/crafting/kinetics/mechanical_pump`. The casing is in the chunk mesh; the block entity
draws the cog (`AllPartialModels.MECHANICAL_PUMP_COG`).
"""

from gen_content import FACING_STATE, block, facing_shapes

COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}
STONE_COLOR = 11  # `MapColor.STONE`

_pump = block(FACING_STATE, {"facing": "north"}, COPPER, STONE_COLOR,
              facing_shapes([[2 / 16, 0, 2 / 16, 14 / 16, 1, 14 / 16]]), render_shape="MODEL")

BLOCKS = {"mechanical_pump": _pump}
PICKAXE_ONLY = {"mechanical_pump"}
NAMES = {"block.create.mechanical_pump": "Mechanical Pump"}
TAB = ["mechanical_pump"]
MODELS = ["block/mechanical_pump/cog"]  # `AllPartialModels.MECHANICAL_PUMP_COG`

RECIPES = {
    "mechanical_pump": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:cogwheel", "create:fluid_pipe"],
        "result": {"id": "create:mechanical_pump"},
    },
}
