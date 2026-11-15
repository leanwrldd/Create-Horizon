"""The Steam Engine and the Powered Shaft: `create:steam_engine`, `create:powered_shaft`.

Create: `AllBlocks.STEAM_ENGINE` (`SharedProperties.copperMetal`, `pickaxeOnly`,
`CStress.setCapacity(1024.0)`, `FaceAttachedHorizontalDirectionalBlock`: `face`, `facing`),
`AllBlocks.POWERED_SHAFT` (`SharedProperties.stone`, `MapColor.METAL`, `pickaxeOnly`, drops a Shaft,
no item), `AllShapes.STEAM_ENGINE`, `STEAM_ENGINE_CEILING`, `STEAM_ENGINE_WALL` and
`EIGHT_VOXEL_POLE`, `recipe/crafting/kinetics/steam_engine`. The engine's block model is in the
chunk mesh; its block entity draws the piston, linkage and crank connector
(`AllPartialModels.ENGINE_*`). The Powered Shaft is drawn by its block entity.
Waterlogging is left out, as for the other blocks.
"""

from gen_content import ANDESITE, AXIS_STATE, METAL, axis_shapes, block

ORANGE = 15  # `Blocks.COPPER_BLOCK`'s `MapColor.COLOR_ORANGE`
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}

# `AttachFace` and `HorizontalDirectionalBlock.FACING`.
FACES = ["floor", "wall", "ceiling"]
FACINGS = ["north", "south", "west", "east"]
STATES = {
    "face": {"type": "enum", "values": FACES},
    "facing": {"type": "enum", "values": FACINGS},
}
DEFAULT = {"face": "floor", "facing": "north"}


def _px(box):
    return [v / 16 for v in box]


def _swap_xz(box):
    """A box turned a quarter about y (every shape here is symmetric about the half turn)."""
    x0, y0, z0, x1, y1, z1 = box
    return [z0, y0, x0, z1, y1, x1]


def _turn_to(box, facing):
    """A box written for a facing of south, turned about y so that it faces `facing`
    (`VoxelShaper.forHorizontal(shape, SOUTH)`)."""
    def rotate(dx, dz):
        return {"south": (dx, dz), "west": (-dz, dx), "north": (-dx, -dz), "east": (dz, -dx)}[facing]
    x0, y0, z0, x1, y1, z1 = box
    corners = [rotate(x - 0.5, z - 0.5) for x, z in ((x0, z0), (x1, z1))]
    xs = sorted(0.5 + c[0] for c in corners)
    zs = sorted(0.5 + c[1] for c in corners)
    return [xs[0], y0, zs[0], xs[1], y1, zs[1]]


# `AllShapes.STEAM_ENGINE`: `forHorizontalAxis()` with Z as the written axis, indexed by the axis
# of `facing`; the ceiling's the same upside down; the wall's `forHorizontal(SOUTH)`.
FLOOR = [_px(b) for b in ([1, 0, 1, 15, 3, 15], [3, 0, 3, 13, 15, 13], [1, 5, 4, 15, 13, 12])]
CEILING = [_px(b) for b in ([1, 13, 1, 15, 16, 15], [3, 1, 3, 13, 16, 13], [1, 3, 4, 15, 11, 12])]
WALL = [_px(b) for b in ([1, 1, 0, 15, 15, 3], [3, 3, 0, 13, 13, 15], [1, 4, 5, 15, 12, 13])]

_values = {}
for _facing in FACINGS:
    along_x = _facing in ("west", "east")
    _values[f"floor,{_facing}"] = [_swap_xz(b) if along_x else b for b in FLOOR]
    _values[f"ceiling,{_facing}"] = [_swap_xz(b) if along_x else b for b in CEILING]
    _values[f"wall,{_facing}"] = [_turn_to(b, _facing) for b in WALL]
SHAPES = {"properties": ["face", "facing"], "values": _values}

_engine = block(STATES, DEFAULT, COPPER, ORANGE, SHAPES, render_shape="MODEL")
_engine["properties"]["occludes"] = False

# `EIGHT_VOXEL_POLE`: the shaft's 8 texel square, along the axis.
_shaft = block(AXIS_STATE, {"axis": "y"}, ANDESITE, METAL,
               axis_shapes([[4 / 16, 0, 4 / 16, 12 / 16, 1, 12 / 16]]))
_shaft["item"] = False  # an engine's shaft turns into it

BLOCKS = {"steam_engine": _engine, "powered_shaft": _shaft}
PICKAXE_ONLY = {"steam_engine", "powered_shaft"}
NAMES = {
    "block.create.steam_engine": "Steam Engine",
    "block.create.powered_shaft": "Powered Shaft",
}
TAB = ["steam_engine"]
# `.loot((lt, block) -> lt.dropOther(block, AllBlocks.SHAFT.get()))`.
LOOT = {"powered_shaft": [{
    "rolls": 1,
    "entries": [{"type": "minecraft:item", "name": "create:shaft"}],
    "condition": {"type": "minecraft:survives_explosion"},
}]}
# `AllPartialModels.ENGINE_PISTON`, `ENGINE_LINKAGE`, `ENGINE_CONNECTOR`, and the boiler's
# `BOILER_GAUGE` and `BOILER_GAUGE_DIAL` (`steam_engine_boiler.rs` draws them on the tank).
MODELS = [
    "block/steam_engine/piston",
    "block/steam_engine/linkage",
    "block/steam_engine/shaft_connector",
    "block/steam_engine/gauge",
    "block/steam_engine/gauge_dial",
]

RECIPES = {
    # `recipe/crafting/kinetics/steam_engine`.
    "steam_engine": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["P", "A", "C"],
        "key": {"P": "#c:plates/gold", "A": "create:andesite_alloy", "C": "#c:storage_blocks/copper"},
        "result": {"id": "create:steam_engine"},
    },
}
# What NeoForge's `c:storage_blocks/copper` holds of vanilla's.
TAGS = {"c:item/storage_blocks/copper": ["minecraft:copper_block"]}
