"""The Smart Fluid Pipe and the Encased Fluid Pipe: `create:smart_fluid_pipe`,
`create:encased_fluid_pipe`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.SMART_FLUID_PIPE` (`SharedProperties.copperMetal`,
`MapColor.TERRACOTTA_YELLOW`, pickaxe only), `SmartFluidPipeBlock` (`FaceAttachedHorizontalDirectionalBlock`:
`face` and `facing`; `AllShapes.SMART_FLUID_PIPE_FLOOR`, `_WALL` and `_CEILING`),
`recipe/crafting/kinetics/smart_fluid_pipe`; `ENCASED_FLUID_PIPE` (`copperMetal`, `noOcclusion`,
`MapColor.TERRACOTTA_LIGHT_GRAY`, `.loot(dropOther(FLUID_PIPE))`, no item) and `EncasedPipeBlock` (the six
sides of a pipe; a full block).

The shapes leave out the one box of each that reaches past the block (17/16 high, `forHorizontal(SOUTH)`).
"""

from gen_content import HORIZONTAL_FACING_STATE, block

COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}  # SharedProperties.copperMetal
YELLOW_TERRACOTTA, LIGHT_GRAY_TERRACOTTA = 40, 44
# `Iterate.directions`.
SIDES = ["down", "up", "north", "south", "west", "east"]

# `AllShapes.SMART_FLUID_PIPE_*`, in texels, written for facing south.
_FLOOR = [(4, 4, 0, 12, 12, 16), (3, 3, 3, 13, 13, 13), (5, 13, 3, 11, 14, 11), (5, 14, 4, 11, 15, 10),
          (5, 15, 5, 11, 16, 9)]  # + (5, 16, 6, 11, 17, 8), outside the block
_WALL = [(4, 0, 4, 12, 16, 12), (3, 3, 3, 13, 13, 13), (5, 5, 13, 11, 13, 14), (5, 6, 14, 11, 12, 15),
         (5, 7, 15, 11, 11, 16)]  # + (5, 8, 16, 11, 10, 17)
_CEILING = [(4, 4, 0, 12, 12, 16), (3, 3, 3, 13, 13, 13), (5, 2, 3, 11, 3, 11), (5, 1, 4, 11, 2, 10),
            (5, 0, 5, 11, 1, 9)]  # + (5, -1, 6, 11, 0, 8)
_BY_FACE = {"floor": _FLOOR, "wall": _WALL, "ceiling": _CEILING}


def _turned(box, facing):
    """A box written for facing south, turned about the block's centre so south points `facing`
    (`VoxelShaper.forHorizontal(SOUTH)`)."""
    x0, y0, z0, x1, y1, z1 = (v / 16 for v in box)

    def point(x, z):
        x, z = x - 0.5, z - 0.5
        x, z = {"south": (x, z), "east": (z, -x), "north": (-x, -z), "west": (-z, x)}[facing]
        return x + 0.5, z + 0.5

    (ax, az), (bx, bz) = point(x0, z0), point(x1, z1)
    return [min(ax, bx), y0, min(az, bz), max(ax, bx), y1, max(az, bz)]


_shapes = {"properties": ["face", "facing"], "values": {
    f"{face},{facing}": [_turned(box, facing) for box in boxes]
    for face, boxes in _BY_FACE.items()
    for facing in ("north", "south", "east", "west")}}

_smart_states = {"face": {"type": "enum", "values": ["floor", "wall", "ceiling"]},
                 **HORIZONTAL_FACING_STATE}
# The body is in the chunk mesh (`create:block/smart_fluid_pipe/block`).
_smart = block(_smart_states, {"face": "floor", "facing": "north"}, COPPER, YELLOW_TERRACOTTA,
               _shapes, render_shape="MODEL")
_smart["properties"]["occludes"] = False

# `EncasedPipeBlock`: a full block, the pipe's six sides open or not, no item (a Copper Casing on a
# pipe makes it).
_encased = block({side: {"type": "boolean"} for side in SIDES}, {side: "false" for side in SIDES},
                 COPPER, LIGHT_GRAY_TERRACOTTA, None, render_shape="MODEL")
_encased["properties"]["occludes"] = False
_encased["item"] = False

BLOCKS = {"smart_fluid_pipe": _smart, "encased_fluid_pipe": _encased}
PICKAXE_ONLY = {"smart_fluid_pipe"}  # the encased pipe is `axeOrPickaxe()`
TAB = ["smart_fluid_pipe"]
NAMES = {
    "block.create.smart_fluid_pipe": "Smart Fluid Pipe",
    "block.create.encased_fluid_pipe": "Encased Fluid Pipe",
}
# `.loot((p, b) -> p.dropOther(b, FLUID_PIPE.get()))`.
LOOT = {"encased_fluid_pipe": [{
    "rolls": 1,
    "entries": [{"type": "minecraft:item", "name": "create:fluid_pipe"}],
    "condition": {"type": "minecraft:survives_explosion"},
}]}

RECIPES = {
    # `recipe/crafting/kinetics/smart_fluid_pipe`.
    "crafting/kinetics/smart_fluid_pipe": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["I", "S", "P"],
        "key": {"I": "#c:plates/brass", "P": "create:electron_tube", "S": "create:fluid_pipe"},
        "result": {"id": "create:smart_fluid_pipe", "count": 1},
    },
}
