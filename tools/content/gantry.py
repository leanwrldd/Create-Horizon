"""The Gantry Shaft and the Gantry Carriage (module `gantry`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.GANTRY_SHAFT` (`SharedProperties.stone`,
`MapColor.NETHER`, axe or pickaxe, `GantryShaftBlock`: `facing`, `part`, `powered`; drawn by its
block entity, `ENTITYBLOCK_ANIMATED`), `GANTRY_CARRIAGE` (`SharedProperties.stone`,
`MapColor.PODZOL`, `noOcclusion`, axe or pickaxe, `GantryCarriageBlock`: `facing`,
`axis_along_first`), `AllShapes.EIGHT_VOXEL_POLE` and the recipes under
`data/create/recipe/crafting/kinetics/`. Neither has a stress value in Create. The behaviour is
`native/api/src/machines/gantry.rs`.
"""

from gen_content import ANDESITE, AXES, FACING_STATE, FACINGS, PODZOL, block

NETHER = 35

SHAFT_STATES = {
    **FACING_STATE,
    "part": {"type": "enum", "values": ["start", "middle", "end", "single"]},
    "powered": {"type": "boolean"},
}
SHAFT_DEFAULT = {"facing": "north", "part": "single", "powered": "false"}
CARRIAGE_STATES = {**FACING_STATE, "axis_along_first": {"type": "boolean"}}
CARRIAGE_DEFAULT = {"facing": "north", "axis_along_first": "false"}

# `AllShapes.EIGHT_VOXEL_POLE`: shape(4, 0, 4, 12, 16, 12) along the shaft's axis.
_POLE = {
    "y": [4 / 16, 0, 4 / 16, 12 / 16, 1, 12 / 16],
    "z": [4 / 16, 4 / 16, 0, 12 / 16, 12 / 16, 1],
    "x": [0, 4 / 16, 4 / 16, 1, 12 / 16, 12 / 16],
}
_AXIS_OF = {"up": "y", "down": "y", "north": "z", "south": "z", "east": "x", "west": "x"}
SHAFT_SHAPES = {"property": "facing", "values": {f: [_POLE[_AXIS_OF[f]]] for f in FACINGS}}

_shaft = block(SHAFT_STATES, SHAFT_DEFAULT, ANDESITE, NETHER, SHAFT_SHAPES)
_shaft["properties"]["occludes"] = False
_carriage = block(CARRIAGE_STATES, CARRIAGE_DEFAULT, ANDESITE, PODZOL, render_shape="MODEL")
_carriage["properties"]["occludes"] = False  # `noOcclusion()`

BLOCKS = {"gantry_shaft": _shaft, "gantry_carriage": _carriage}

TAB = ["gantry_carriage", "gantry_shaft"]

NAMES = {
    "block.create.gantry_shaft": "Gantry Shaft",
    "block.create.gantry_carriage": "Gantry Carriage",
}

RECIPES = {
    # `crafting/kinetics/gantry_shaft` and `gantry_carriage`, in Horizon's 26.3 form.
    "crafting/kinetics/gantry_shaft": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["A", "R", "A"],
        "key": {"A": "create:andesite_alloy", "R": "#c:dusts/redstone"},
        "result": {"id": "create:gantry_shaft", "count": 8},
    },
    "crafting/kinetics/gantry_carriage": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "#minecraft:wooden_slabs", "C": "create:andesite_casing",
                "I": "create:cogwheel"},
        "result": {"id": "create:gantry_carriage", "count": 1},
    },
}
