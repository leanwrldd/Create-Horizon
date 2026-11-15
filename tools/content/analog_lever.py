"""The Analog Lever: `create:analog_lever`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ANALOG_LEVER` (`initialProperties(() -> Blocks.LEVER)`:
`strength(0.5)`, `noCollission`, `SoundType.STONE`; `axeOrPickaxe`; tag `SAFE_NBT`; blockstate
`horizontalFaceBlock` with `analog_lever/block`), `AnalogLeverBlock` (`face`, `facing` of
`FaceAttachedHorizontalDirectionalBlock`; the shape is vanilla's lever's),
`data/create/recipe/crafting/kinetics/analog_lever.json`. The base is in the chunk mesh from
Create's blockstate file; the block entity draws the handle (`AllPartialModels.ANALOG_LEVER_HANDLE`)
and the indicator. The behaviour is `native/api/src/machines/analog_lever.rs`.
"""
from gen_content import block

# Vanilla `LeverBlock` boxes, in texels: `UP_AABB_Z`/`X`, `DOWN_AABB_Z`/`X` and the four walls.
_FLOOR = {"z": [5, 0, 4, 11, 6, 12], "x": [4, 0, 5, 12, 6, 11]}
_CEILING = {"z": [5, 10, 4, 11, 16, 12], "x": [4, 10, 5, 12, 16, 11]}
_WALL = {"north": [5, 4, 10, 11, 12, 16], "south": [5, 4, 0, 11, 12, 6],
         "west": [10, 4, 5, 16, 12, 11], "east": [0, 4, 5, 6, 12, 11]}


def _blocks(box):
    return [[round(v / 16, 6) for v in box]]


def _shapes():
    values = {}
    for facing in ("north", "south", "west", "east"):
        axis = "x" if facing in ("east", "west") else "z"
        values[f"floor,{facing}"] = _blocks(_FLOOR[axis])
        values[f"ceiling,{facing}"] = _blocks(_CEILING[axis])
        values[f"wall,{facing}"] = _blocks(_WALL[facing])
    return {"properties": ["face", "facing"], "values": values, "default": values["wall,north"]}


# `Blocks.LEVER`: `strength(0.5F)`, `SoundType.STONE` (named by a block), no map colour.
LEVER = {"hardness": 0.5, "explosion_resistance": 0.5, "requires_tool": False,
         "sounds": "minecraft:stone"}

_lever = block(
    {"face": {"type": "enum", "values": ["floor", "wall", "ceiling"]},
     "facing": {"type": "enum", "values": ["north", "south", "west", "east"]}},
    {"face": "wall", "facing": "north"}, LEVER, 0, _shapes(), render_shape="MODEL")
_lever["collision_shapes"] = {"default": []}  # `noCollission`

BLOCKS = {"analog_lever": _lever}

# `AllPartialModels.ANALOG_LEVER_HANDLE`: the indicator is drawn as a tinted box.
MODELS = ["block/analog_lever/handle"]

NAMES = {"block.create.analog_lever": "Analog Lever"}

RECIPES = {
    # `crafting/kinetics/analog_lever`.
    "crafting/kinetics/analog_lever": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["P", "S"],
        "key": {"P": "#c:rods/wooden", "S": "create:andesite_casing"},
        "result": {"id": "create:analog_lever", "count": 1},
    },
}

TAGS = {"create:block/safe_nbt": ["create:analog_lever"]}

TAB = ["analog_lever"]
