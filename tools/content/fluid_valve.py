"""The Fluid Valve and the valve handles: `create:fluid_valve`, `create:copper_valve_handle` and
`create:<colour>_valve_handle`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.FLUID_VALVE` (`SharedProperties.copperMetal`, pickaxe only,
`DirectionalAxisKineticBlock`'s `facing` and `axis_along_first` and its own `enabled`;
`AllShapes.FLUID_VALVE` for `getPipeAxis`), `COPPER_VALVE_HANDLE` and `DYED_VALVE_HANDLES`
(`BuilderTransformers.valveHandle`: `ValveHandleBlock`, a `HandCrankBlock` with `AllShapes.VALVE_HANDLE`
and `ENTITYBLOCK_ANIMATED`, tags `create:brittle` and `create:valve_handles`, `CStress.setCapacity(8.0)`),
`AllPartialModels.FLUID_VALVE_POINTER`, the datagen recipes `crafting/kinetics/fluid_valve`,
`copper_valve_handle`, `copper_valve_handle_from_others` and `<colour>_valve_handle_from_other_valve_handle`.
"""

from gen_content import FACING_STATE, block, facing_shapes, turn

ORANGE = 15  # `Blocks.COPPER_BLOCK`'s `MapColor.COLOR_ORANGE`
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}  # SharedProperties.copperMetal
AXES = ["x", "y", "z"]
FACINGS = ["north", "south", "east", "west", "up", "down"]
FACING_AXIS = {"up": "y", "down": "y", "north": "z", "south": "z", "east": "x", "west": "x"}

# `DyeColor`: `(name, MapColor id of getMapColor())`.
COLOURS = [("white", 8), ("orange", 15), ("magenta", 16), ("light_blue", 17), ("yellow", 18),
           ("lime", 19), ("pink", 20), ("gray", 21), ("light_gray", 22), ("cyan", 23),
           ("purple", 24), ("blue", 25), ("brown", 26), ("green", 27), ("red", 28), ("black", 29)]


def _pipe_axis(facing, along_first):
    """`FluidValveBlock.getPipeAxis`: Create's local `alongFirst` is the property inverted."""
    skip = along_first == "true"
    for axis in AXES:
        if axis == FACING_AXIS[facing]:
            continue
        if skip:
            skip = False
            continue
        return axis
    raise ValueError(facing)


# `AllShapes.FLUID_VALVE`: shape(3, 0, 3, 13, 16, 13).add(2, 2, 2, 14, 14, 14).forAxis(), by the pipe axis.
_VALVE = [[3 / 16, 0, 3 / 16, 13 / 16, 1, 13 / 16], [2 / 16, 2 / 16, 2 / 16, 14 / 16, 14 / 16, 14 / 16]]
_shapes = {"properties": ["facing", "axis_along_first"], "values": {}}
for _facing in FACINGS:
    for _first in ("false", "true"):
        _axis = _pipe_axis(_facing, _first)
        _shapes["values"][f"{_facing},{_first}"] = [turn(box, _axis) for box in _VALVE]

_valve_states = {**FACING_STATE, "axis_along_first": {"type": "boolean"},
                 "enabled": {"type": "boolean"}}
# Its casing is in the chunk mesh (`block_<horizontal|vertical>_<open|closed>`); the block entity
# draws the shaft and the pointer.
_valve = block(_valve_states, {"facing": "north", "axis_along_first": "false", "enabled": "false"},
               COPPER, ORANGE, _shapes, render_shape="MODEL")
_valve["properties"]["occludes"] = False

# `AllShapes.VALVE_HANDLE`: shape(5, 0, 5, 11, 4, 11).add(1, 3, 1, 15, 8, 15), facing up.
_HANDLE = [[5 / 16, 0, 5 / 16, 11 / 16, 4 / 16, 11 / 16], [1 / 16, 3 / 16, 1 / 16, 15 / 16, 8 / 16, 15 / 16]]


def _handle(map_color):
    # The block entity draws the handle itself, turned about its facing.
    return block(FACING_STATE, {"facing": "up"}, COPPER, map_color, facing_shapes(_HANDLE))


HANDLES = ["copper_valve_handle", *(f"{name}_valve_handle" for name, _ in COLOURS)]
BLOCKS = {
    "fluid_valve": _valve,
    "copper_valve_handle": _handle(ORANGE),
    **{f"{name}_valve_handle": _handle(color) for name, color in COLOURS},
}
PICKAXE_ONLY = set(BLOCKS)
TAB = ["fluid_valve", *HANDLES]
MODELS = ["block/fluid_valve/pointer"]

NAMES = {
    "block.create.fluid_valve": "Fluid Valve",
    "block.create.copper_valve_handle": "Copper Valve Handle",
    **{f"block.create.{name}_valve_handle": f"{name.replace('_', ' ').title()} Valve Handle"
       for name, _ in COLOURS},
    "tag.block.create.valve_handles": "Valve Handles",
    "tag.item.create.valve_handles": "Valve Handles",
}

TAGS = {
    # `BuilderTransformers.valveHandle`: `.tag(AllBlockTags.BRITTLE.tag, AllBlockTags.VALVE_HANDLES.tag)`
    # on the block and `AllItemTags.VALVE_HANDLES.tag` on the item.
    "create:block/valve_handles": [f"create:{h}" for h in HANDLES],
    "create:item/valve_handles": [f"create:{h}" for h in HANDLES],
    "create:block/brittle": [f"create:{h}" for h in HANDLES],
    # Vanilla's dyes under NeoForge's convention tags the recipes below name (vanilla has no `c:`).
    **{f"c:item/dyes/{name}": [f"minecraft:{name}_dye"] for name, _ in COLOURS},
}

RECIPES = {
    # `recipe/crafting/kinetics/fluid_valve`.
    "crafting/kinetics/fluid_valve": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["#c:plates/iron", "create:fluid_pipe"],
        "result": {"id": "create:fluid_valve", "count": 1},
    },
    # `recipe/crafting/kinetics/copper_valve_handle`.
    "crafting/kinetics/copper_valve_handle": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["CCC", " S "],
        "key": {"C": "#c:plates/copper", "S": "create:andesite_alloy"},
        "result": {"id": "create:copper_valve_handle", "count": 1},
    },
    # `recipe/crafting/kinetics/copper_valve_handle_from_others`.
    "crafting/kinetics/copper_valve_handle_from_others": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["#create:valve_handles"],
        "result": {"id": "create:copper_valve_handle", "count": 1},
    },
    # `recipe/crafting/kinetics/<colour>_valve_handle_from_other_valve_handle`.
    **{f"crafting/kinetics/{name}_valve_handle_from_other_valve_handle": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": [f"#c:dyes/{name}", "#create:valve_handles"],
        "result": {"id": f"create:{name}_valve_handle", "count": 1},
    } for name, _ in COLOURS},
}
