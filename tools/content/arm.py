"""The Mechanical Arm: `create:mechanical_arm` (module `arm`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.MECHANICAL_ARM` (`SharedProperties.softMetal`,
`MapColor.TERRACOTTA_YELLOW`, `axeOrPickaxe`, `CStress.setImpact(2.0)`, `ArmItem`), `ArmBlock` (the
`ceiling` property; `AllShapes.MECHANICAL_ARM` and `MECHANICAL_ARM_CEILING`), the blockstate
(`ceiling=true` turns the model 180 about x), `AllPartialModels.ARM_COG`, `ARM_BASE`, `ARM_LOWER_BODY`,
`ARM_UPPER_BODY`, `ARM_CLAW_BASE`, `ARM_CLAW_BASE_GOGGLES`, `ARM_CLAW_GRIP_UPPER` and
`ARM_CLAW_GRIP_LOWER`, and `recipe/crafting/kinetics/mechanical_arm`. The loot table is the default
one (Create's drops the block itself). The behaviour is `native/api/src/machines/arm*.rs`.
"""

from gen_content import block

TERRACOTTA_YELLOW = 40  # MapColor.TERRACOTTA_YELLOW
# `SharedProperties.softMetal`: a gold block.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:gold_block"}

# `AllShapes.MECHANICAL_ARM = shape(2, 0, 2, 14, 10, 14).add(3, 0, 3, 13, 14, 13).add(0, 0, 0, 16, 6, 16)`
# and `MECHANICAL_ARM_CEILING = shape(2, 6, 2, 14, 16, 14).add(3, 2, 3, 13, 16, 13).add(0, 10, 0, 16, 16, 16)`.
_FLOOR = [[2 / 16, 0, 2 / 16, 14 / 16, 10 / 16, 14 / 16],
          [3 / 16, 0, 3 / 16, 13 / 16, 14 / 16, 13 / 16],
          [0, 0, 0, 1, 6 / 16, 1]]
_CEILING = [[2 / 16, 6 / 16, 2 / 16, 14 / 16, 1, 14 / 16],
            [3 / 16, 2 / 16, 3 / 16, 13 / 16, 1, 13 / 16],
            [0, 10 / 16, 0, 1, 1, 1]]

BLOCKS = {
    # The base is in the chunk mesh (`block/mechanical_arm/block`); the block entity draws the
    # cog, the arm and what it holds.
    "mechanical_arm": block({"ceiling": {"type": "boolean"}}, {"ceiling": "false"}, SOFT_METAL,
                            TERRACOTTA_YELLOW,
                            {"property": "ceiling", "values": {"false": _FLOOR, "true": _CEILING}},
                            render_shape="MODEL"),
}
# `axeOrPickaxe()`: the default tags (both tools).

# What the block entity draws besides the block's model.
MODELS = ["block/mechanical_arm/cog", "block/mechanical_arm/base", "block/mechanical_arm/lower_body",
          "block/mechanical_arm/upper_body", "block/mechanical_arm/claw_base",
          "block/mechanical_arm/claw_base_goggles", "block/mechanical_arm/upper_claw_grip",
          "block/mechanical_arm/lower_claw_grip"]

TAB = ["mechanical_arm"]

NAMES = {"block.create.mechanical_arm": "Mechanical Arm"}

RECIPES = {
    # `recipe/crafting/kinetics/mechanical_arm`, in Horizon's 26.3 form.
    "crafting/kinetics/mechanical_arm": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["LLA", "L  ", "IC "],
        "key": {"A": "create:andesite_alloy", "C": "create:brass_casing",
                "I": "create:precision_mechanism", "L": "#c:plates/brass"},
        "result": {"id": "create:mechanical_arm", "count": 1},
    },
}
