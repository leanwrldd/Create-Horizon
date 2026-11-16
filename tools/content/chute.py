"""The Chute and the Smart Chute: `create:chute` and `create:smart_chute`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.CHUTE` (`SharedProperties.softMetal`,
`MapColor.COLOR_GRAY`, `SoundType.NETHERITE_BLOCK`, `noOcclusion`, `pickaxeOnly`) with `ChuteBlock`
(`facing`: `FACING_HOPPER`, and `shape`: `ChuteBlock.Shape`), `ChuteShapes` (`AllShapes.CHUTE`,
`CHUTE_SLOPE`), `ChuteGenerator`, `AllBlocks.SMART_CHUTE` (the same, `SmartChuteBlock.POWERED`
true by default), `recipe/crafting/kinetics/chute` and `smart_chute`. The blocks are in the chunk
mesh (Create's `block/chute/*` models, picked by the imported blockstates); the block entity draws
the item falling through a window. The behaviour is `native/api/src/machines/chute.rs`.

Shapes are cut to the block's own cell: Create's slope panels reach 19/16 high, and its
collision mask lets them stand up to 1.5 (`ChuteShapes.COLLISION_MASK`).
"""

from gen_content import GRAY, block

# `SharedProperties.softMetal`: a gold block. `SoundType.NETHERITE_BLOCK` replaces its sound.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:gold_block"}

# `BlockStateProperties.FACING_HOPPER`: down and the four horizontal sides.
FACINGS = ["down", "north", "south", "west", "east"]
# `ChuteBlock.Shape`.
SHAPES = ["intersection", "window", "normal", "encased"]

# `AllShapes.CHUTE = shape(1, 8, 1, 15, 16, 15).add(2, 0, 2, 14, 8, 14)`, in blocks.
_CHUTE = [[1 / 16, 8 / 16, 1 / 16, 15 / 16, 1, 15 / 16], [2 / 16, 0, 2 / 16, 14 / 16, 8 / 16, 14 / 16]]
_BLOCK = [[0, 0, 0, 1, 1, 1]]


def _turn(box, quarter_turns):
    """`box` turned clockwise seen from above (north to east) about the block's centre."""
    x0, y0, z0, x1, y1, z1 = box
    corners = [(x0 - 0.5, z0 - 0.5), (x1 - 0.5, z1 - 0.5)]
    for _ in range(quarter_turns):
        corners = [(-dz, dx) for dx, dz in corners]
    (ax, az), (bx, bz) = corners
    return [round(min(ax, bx) + 0.5, 6), y0, round(min(az, bz) + 0.5, 6),
            round(max(ax, bx) + 0.5, 6), y1, round(max(az, bz) + 0.5, 6)]


def _slope():
    """`ChuteShapes.createSlope`: sixteen panels, `PANEL = box(1, -15, 0, 15, 4, 1)` moved up and
    back a texel each, written for a chute facing south (it falls towards the north)."""
    boxes = []
    for i in range(16):
        lo, hi = max(0.0, (-15 + i) / 16), min(1.0, (4 + i) / 16)
        boxes.append([1 / 16, lo, i / 16, 15 / 16, hi, (i + 1) / 16])
    return boxes


# The blockstate turns the model clockwise about y by `horizontalAngle(facing)`: south 0, west
# 90, north 180, east 270 (`ChuteGenerator.getYRotation`), and the shape follows it.
_TURNS = {"south": 0, "west": 1, "north": 2, "east": 3}


def _shape_of(facing, shape):
    """`ChuteShapes.createShape`: an intersection or an encased chute is a full block, a plain
    one is the chute's tube (down) or the slope (the four sides)."""
    solid = shape in ("intersection", "encased")
    if facing == "down":
        return _BLOCK if solid else _CHUTE
    if solid:
        return _BLOCK
    return [_turn(box, _TURNS[facing]) for box in _slope()]


_SHAPES = {"properties": ["facing", "shape"], "values": {
    f"{facing},{shape}": _shape_of(facing, shape) for facing in FACINGS for shape in SHAPES}}

_chute = block(
    {"facing": {"type": "enum", "values": FACINGS}, "shape": {"type": "enum", "values": SHAPES}},
    {"facing": "down", "shape": "normal"}, SOFT_METAL, GRAY, _SHAPES,
    sounds="minecraft:netherite_block", render_shape="MODEL")
_chute["properties"]["occludes"] = False  # `noOcclusion`

# `SmartChuteBlock`: `Shapes.block()` (`ChuteShapes.createShape`). Create registers `POWERED = true`
# but `getStateForPlacement` always sets it from `hasNeighborSignal`; Horizon's placement hook can't
# read the neighbours (`chute.rs` `place_block` and the poll fix it), so the state a chute is placed
# in is the one nearly every placement ends in: unpowered. A default of `true` read as powered for
# a tick or two after every placement.
_smart = block({"powered": {"type": "boolean"}}, {"powered": "false"}, SOFT_METAL, GRAY,
               sounds="minecraft:netherite_block", render_shape="MODEL")
_smart["properties"]["occludes"] = False

BLOCKS = {"chute": _chute, "smart_chute": _smart}
PICKAXE_ONLY = {"chute", "smart_chute"}
TAB = ["chute", "smart_chute"]

NAMES = {
    "block.create.chute": "Chute",
    "block.create.smart_chute": "Smart Chute",
}

# `CreateStandardRecipeGen` (`crafting/kinetics/chute`, `smart_chute`) in Horizon's 26.3 form.
RECIPES = {
    "crafting/kinetics/chute": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["A", "I", "A"],
        "key": {"A": "#c:plates/iron", "I": "#c:ingots/iron"},
        "result": {"id": "create:chute", "count": 4},
    },
    "crafting/kinetics/smart_chute": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["I", "S", "P"],
        "key": {"I": "#c:plates/brass", "P": "create:electron_tube", "S": "create:chute"},
        "result": {"id": "create:smart_chute", "count": 1},
    },
}
