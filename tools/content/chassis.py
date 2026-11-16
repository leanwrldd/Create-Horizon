"""The Linear Chassis, the Secondary Linear Chassis and the Radial Chassis (module `chassis`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.LINEAR_CHASSIS` / `SECONDARY_LINEAR_CHASSIS` /
`RADIAL_CHASSIS` (`SharedProperties.wooden`, `MapColor.TERRACOTTA_BROWN` / `PODZOL` / `DIRT`, axe
or pickaxe), `AbstractChassisBlock` (`axis`), `LinearChassisBlock` (`sticky_top`,
`sticky_bottom`), `RadialChassisBlock` (`sticky_north`, `sticky_south`, `sticky_east`,
`sticky_west`; waterlogging is left out, as elsewhere), and the recipes under
`data/create/recipe/crafting/kinetics/`.
"""

from gen_content import AXIS_STATE, DIRT, WOODEN, block

TERRACOTTA_BROWN, PODZOL = 48, 34

LINEAR_STATES = {
    **AXIS_STATE,
    "sticky_top": {"type": "boolean"},
    "sticky_bottom": {"type": "boolean"},
}
LINEAR_DEFAULT = {"axis": "y", "sticky_top": "false", "sticky_bottom": "false"}
RADIAL_STATES = {
    **AXIS_STATE,
    "sticky_north": {"type": "boolean"},
    "sticky_south": {"type": "boolean"},
    "sticky_east": {"type": "boolean"},
    "sticky_west": {"type": "boolean"},
}
RADIAL_DEFAULT = {"axis": "y", "sticky_north": "false", "sticky_south": "false",
                  "sticky_east": "false", "sticky_west": "false"}

BLOCKS = {
    "linear_chassis": block(LINEAR_STATES, LINEAR_DEFAULT, WOODEN, TERRACOTTA_BROWN,
                            render_shape="MODEL"),
    "secondary_linear_chassis": block(LINEAR_STATES, LINEAR_DEFAULT, WOODEN, PODZOL,
                                      render_shape="MODEL"),
    "radial_chassis": block(RADIAL_STATES, RADIAL_DEFAULT, WOODEN, DIRT, render_shape="MODEL"),
}

TAB = ["linear_chassis", "secondary_linear_chassis", "radial_chassis"]

NAMES = {
    "block.create.linear_chassis": "Linear Chassis",
    "block.create.secondary_linear_chassis": "Secondary Linear Chassis",
    "block.create.radial_chassis": "Radial Chassis",
}

RECIPES = {
    # `crafting/kinetics/linear_chassis`, `radial_chassis` and the two conversions, in Horizon's
    # 26.3 form.
    "crafting/kinetics/linear_chassis": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" P ", "LLL", " P "],
        "key": {"L": "#minecraft:logs", "P": "create:andesite_alloy"},
        "result": {"id": "create:linear_chassis", "count": 3},
    },
    "crafting/kinetics/radial_chassis": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" L ", "PLP", " L "],
        "key": {"L": "#minecraft:logs", "P": "create:andesite_alloy"},
        "result": {"id": "create:radial_chassis", "count": 3},
    },
    "crafting/kinetics/linear_chassis_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:secondary_linear_chassis"],
        "result": {"id": "create:linear_chassis", "count": 1},
    },
    "crafting/kinetics/secondary_linear_chassis_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:linear_chassis"],
        "result": {"id": "create:secondary_linear_chassis", "count": 1},
    },
}
