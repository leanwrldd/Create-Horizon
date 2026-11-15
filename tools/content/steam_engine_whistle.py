"""The Steam Whistle and its extension: `create:steam_whistle`, `create:steam_whistle_extension`.

Create: `AllBlocks.STEAM_WHISTLE` (`SharedProperties.copperMetal`, `MapColor.GOLD`, `pickaxeOnly`;
`WhistleBlock`: `facing`, `powered`, `size`, `wall`), `STEAM_WHISTLE_EXTENSION` (`WhistleExtenderBlock`:
`shape` and `size`; no item, no drops), `AllShapes.WHISTLE_*`, `recipe/crafting/kinetics/steam_whistle`.
Both are in the chunk mesh (Create's blockstates: the powered whistle has its own texture); the
whistle's block entity draws the mouth (`AllPartialModels.WHISTLE_MOUTH_*`). The extension has a
block entity of this mod's own, only to keep itself standing.
"""

from gen_content import block

GOLD = 30  # `MapColor.GOLD`
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}

FACINGS = ["north", "south", "west", "east"]
SIZES = ["small", "medium", "large"]
STATES = {
    "facing": {"type": "enum", "values": FACINGS},
    "wall": {"type": "boolean"},
    "powered": {"type": "boolean"},
    "size": {"type": "enum", "values": SIZES},
}
DEFAULT = {"facing": "north", "wall": "false", "powered": "false", "size": "medium"}


def _px(box):
    return [v / 16 for v in box]


def _turn_to(box, facing):
    """A box written for a facing of south, turned about y to `facing`
    (`VoxelShaper.forHorizontal(shape, SOUTH)`)."""
    def rotate(dx, dz):
        return {"south": (dx, dz), "west": (-dz, dx), "north": (-dx, -dz), "east": (dz, -dx)}[facing]
    x0, y0, z0, x1, y1, z1 = box
    corners = [rotate(x - 0.5, z - 0.5) for x, z in ((x0, z0), (x1, z1))]
    xs = sorted(0.5 + c[0] for c in corners)
    zs = sorted(0.5 + c[1] for c in corners)
    return [xs[0], y0, zs[0], xs[1], y1, zs[1]]


# `AllShapes.WHISTLE_SMALL`, `_MEDIUM`, `_LARGE`.
BODY = {"small": _px([4, 3, 4, 12, 16, 12]), "medium": _px([3, 3, 3, 13, 16, 13]),
        "large": _px([2, 3, 2, 14, 16, 14])}
# `WHISTLE_BASE` facing up (`shape(1, 0, 1, 15, 3, 15).add(5, 0, 5, 11, 8, 11)`) ...
BASE_FLOOR = [_px([1, 0, 1, 15, 3, 15]), _px([5, 0, 5, 11, 8, 11])]
# ... and turned to point north, as the wall shape adds it for a whistle facing south: the plate
# against the tank at the south side, the pipe reaching out of it.
BASE_WALL = [[1 / 16, 1 / 16, 13 / 16, 15 / 16, 15 / 16, 1.0],
             [5 / 16, 5 / 16, 8 / 16, 11 / 16, 11 / 16, 1.0]]

_values = {}
for _wall in ("false", "true"):
    for _facing in FACINGS:
        for _size in SIZES:
            if _wall == "false":
                boxes = [BODY[_size], *BASE_FLOOR]
            else:
                boxes = [_turn_to(b, _facing) for b in [BODY[_size], *BASE_WALL]]
            # The `powered` state has the same shape: the table leaves it out.
            _values[f"{_wall},{_facing},{_size}"] = boxes
SHAPES = {"properties": ["wall", "facing", "size"], "values": _values}

_whistle = block(STATES, DEFAULT, COPPER, GOLD, SHAPES, render_shape="MODEL")
_whistle["properties"]["occludes"] = False

# `WhistleExtenderBlock`: `shape` and `size`, shapes by `AllShapes.WHISTLE_EXTENDER_*`.
EXTENSION_STATES = {
    "shape": {"type": "enum", "values": ["single", "double", "double_connected"]},
    "size": {"type": "enum", "values": SIZES},
}
EXTENSION_DEFAULT = {"shape": "single", "size": "medium"}
_edge = {"small": 4, "medium": 3, "large": 2}
_ext_values = {}
for _size, _lo in _edge.items():
    _hi = 16 - _lo
    _ext_values[f"single,{_size}"] = [_px([_lo, 0, _lo, _hi, 10, _hi])]
    _ext_values[f"double,{_size}"] = [_px([_lo, 0, _lo, _hi, 18, _hi])]
    _ext_values[f"double_connected,{_size}"] = [_px([_lo, 0, _lo, _hi, 16, _hi])]
EXTENSION_SHAPES = {"properties": ["shape", "size"], "values": _ext_values}

_extension = block(EXTENSION_STATES, EXTENSION_DEFAULT, COPPER, GOLD, EXTENSION_SHAPES,
                   render_shape="MODEL")
_extension["properties"]["occludes"] = False
_extension["item"] = False  # a Steam Whistle used on a whistle places it, and it drops nothing

BLOCKS = {"steam_whistle": _whistle, "steam_whistle_extension": _extension}
PICKAXE_ONLY = {"steam_whistle", "steam_whistle_extension"}
NAMES = {
    "block.create.steam_whistle": "Steam Whistle",
    "block.create.steam_whistle_extension": "Steam Whistle Extension",
}
TAB = ["steam_whistle"]
# `AllPartialModels.WHISTLE_MOUTH_LARGE`, `_MEDIUM`, `_SMALL`.
MODELS = [
    "block/steam_whistle/small_mouth",
    "block/steam_whistle/medium_mouth",
    "block/steam_whistle/large_mouth",
]

RECIPES = {
    # `recipe/crafting/kinetics/steam_whistle`.
    "steam_whistle": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["P", "C"],
        "key": {"P": "#c:plates/gold", "C": "#c:ingots/copper"},
        "result": {"id": "create:steam_whistle"},
    },
}
