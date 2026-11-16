"""The Fluid Pipe and the Glass Fluid Pipe: `create:fluid_pipe`, `create:glass_fluid_pipe`.

Create: `AllBlocks.FLUID_PIPE` (`SharedProperties.copperMetal`, `forceSolidOff`, `pickaxeOnly`),
`FluidPipeBlock` (a `PipeBlock` with apothem 4/16: `down` .. `east`), `GLASS_FLUID_PIPE`
(`GlassFluidPipeBlock`: `axis` and `alt`, `noOcclusion`, drops a Fluid Pipe, no item of its own),
`recipe/crafting/kinetics/fluid_pipe` and `fluid_pipe_vertical`. Both are in the chunk mesh
(Create's multipart and window models, which hold only the core); the arms, rims and drains are
partial models the block entities draw (`MODELS`). The shapes are written out for each combination
of sides. The Encased Fluid Pipe and the Smart Fluid Pipe are in `smart_fluid_pipe.py`.
"""

import itertools

from gen_content import AXIS_STATE, axis_shapes, block

ORANGE = 15  # `Blocks.COPPER_BLOCK`'s `MapColor.COLOR_ORANGE`
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}
# `Iterate.directions`.
SIDES = ["down", "up", "north", "south", "west", "east"]
NORMALS = {"down": (1, -1), "up": (1, 1), "north": (2, -1), "south": (2, 1),
           "west": (0, -1), "east": (0, 1)}
LO, HI = 4 / 16, 12 / 16  # `PipeBlock(4 / 16f)`


def _boxes(open_sides):
    """The core and an arm to each open side, in blocks."""
    boxes = [[LO, LO, LO, HI, HI, HI]]
    for side in open_sides:
        axis, sign = NORMALS[side]
        box = [LO, LO, LO, HI, HI, HI]
        if sign > 0:
            box[axis + 3] = 1.0
        else:
            box[axis] = 0.0
        boxes.append(box)
    return boxes


_shapes = {"properties": SIDES, "values": {}}
for combo in itertools.product(["false", "true"], repeat=6):
    _shapes["values"][",".join(combo)] = _boxes([s for s, v in zip(SIDES, combo) if v == "true"])

_pipe = block({side: {"type": "boolean"} for side in SIDES}, {side: "false" for side in SIDES},
              COPPER, ORANGE, _shapes, render_shape="MODEL")
_pipe["properties"]["occludes"] = False

_glass = block({**AXIS_STATE, "alt": {"type": "boolean"}}, {"axis": "y", "alt": "false"},
               COPPER, ORANGE, axis_shapes([[LO, 0, LO, HI, 1, HI]]), render_shape="MODEL")
_glass["properties"]["occludes"] = False
_glass["item"] = False  # a wrench turns a pipe into it

BLOCKS = {"fluid_pipe": _pipe, "glass_fluid_pipe": _glass}
PICKAXE_ONLY = {"fluid_pipe", "glass_fluid_pipe"}
NAMES = {
    "block.create.fluid_pipe": "Fluid Pipe",
    "block.create.glass_fluid_pipe": "Glass Fluid Pipe",
}
TAB = ["fluid_pipe"]
# `AllPartialModels.PIPE_ATTACHMENTS` and `FLUID_PIPE_CASING`: what `PipeAttachmentModel` adds to the pipes, the
# glass and encased pipes, the smart pipe, the valve and the pump. The block entities draw them
# (`fluid_pipe_attachments.rs`); the chunk model has only the core.
MODELS = ["block/fluid_pipe/casing"] + [
    f"block/fluid_pipe/{part}/{side}"
    for part in ("connection", "rim_connector", "rim", "drain") for side in SIDES]
# `.loot((p, b) -> p.dropOther(b, FLUID_PIPE.get()))`.
LOOT = {"glass_fluid_pipe": [{
    "rolls": 1,
    "entries": [{"type": "minecraft:item", "name": "create:fluid_pipe"}],
    "condition": {"type": "minecraft:survives_explosion"},
}]}

RECIPES = {
    # `recipe/crafting/kinetics/fluid_pipe` and `fluid_pipe_vertical`: four pipes either way.
    "fluid_pipe": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["SCS"],
        "key": {"C": "#c:ingots/copper", "S": "#c:plates/copper"},
        "result": {"id": "create:fluid_pipe", "count": 4},
    },
    "fluid_pipe_vertical": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["S", "C", "S"],
        "key": {"C": "#c:ingots/copper", "S": "#c:plates/copper"},
        "result": {"id": "create:fluid_pipe", "count": 4},
    },
}
