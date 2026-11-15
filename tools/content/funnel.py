"""The Andesite and Brass Funnels, and the funnels that sit on a belt or a depot:
`create:andesite_funnel`, `create:brass_funnel`, `create:andesite_belt_funnel`,
`create:brass_belt_funnel`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ANDESITE_FUNNEL` (`SharedProperties.stone`,
`MapColor.STONE`, `pickaxeOnly`), `BRASS_FUNNEL` (`SharedProperties.softMetal`,
`MapColor.TERRACOTTA_YELLOW`), `ANDESITE_BELT_FUNNEL` and `BRASS_BELT_FUNNEL` (the same without an
item: their loot drops the funnel), `FunnelBlock` (`facing`, `powered`, `extracting`),
`BeltFunnelBlock` (`facing`, `powered`, `shape`), `FunnelGenerator`, `BeltFunnelGenerator`,
`AllShapes.FUNNEL_*`, `BELT_FUNNEL_*`, `FUNNEL_COLLISION`, `recipe/crafting/logistics/*_funnel`.
The blocks are in the chunk mesh (Create's `block/*_funnel_*` models, picked by the imported
blockstates); the block entity draws the flaps. The behaviour is
`native/api/src/machines/funnel.rs`.

Shapes are cut to the block's own cell (Create's reach 2 to 5 texels outside it).
"""

from gen_content import ANDESITE, FACING_STATE, HORIZONTAL_FACING_STATE, block

STONE = 11  # `MapColor.STONE`
TERRACOTTA_YELLOW = 40  # `MapColor.TERRACOTTA_YELLOW`
# `SharedProperties.softMetal`: a gold block.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:gold_block"}

FACINGS = ["north", "south", "east", "west", "up", "down"]
HORIZONTALS = ["north", "south", "west", "east"]
SHAPES = ["retracted", "extended", "pushing", "pulling"]  # `BeltFunnelBlock.Shape`


def _clamped(boxes):
    """Texel boxes `[x0, y0, z0, x1, y1, z1]` as blocks inside the cell; boxes left with no
    thickness are dropped."""
    out = []
    for box in boxes:
        lo = [min(max(v, 0), 16) / 16 for v in box[:3]]
        hi = [min(max(v, 0), 16) / 16 for v in box[3:]]
        if all(hi[i] > lo[i] for i in range(3)):
            out.append([round(v, 6) for v in lo + hi])
    return out


def _turn(box, quarter_turns):
    """`box` (blocks) turned clockwise seen from above (north to east) about the centre."""
    x0, y0, z0, x1, y1, z1 = box
    corners = [(x0 - 0.5, z0 - 0.5), (x1 - 0.5, z1 - 0.5)]
    for _ in range(quarter_turns):
        corners = [(-dz, dx) for dx, dz in corners]
    (ax, az), (bx, bz) = corners
    return [round(min(ax, bx) + 0.5, 6), y0, round(min(az, bz) + 0.5, 6),
            round(max(ax, bx) + 0.5, 6), y1, round(max(az, bz) + 0.5, 6)]


# `VoxelShaper.forHorizontal(NORTH)`: the shape is written facing north and turned clockwise.
_TURNS = {"north": 0, "east": 1, "south": 2, "west": 3}


def _facing(boxes, facing):
    return [_turn(box, _TURNS[facing]) for box in _clamped(boxes)]


# `AllShapes`, in texels.
_FLOOR = [[2, -2, 2, 14, 8, 14], [1, 1, 1, 15, 8, 15], [0, 4, 0, 16, 10, 16]]
_CEILING = [[2, 8, 2, 14, 18, 14], [1, 8, 1, 15, 15, 15], [0, 6, 0, 16, 12, 16]]
_WALL = [[2, 2, 14, 14, 14, 18], [1, 8, 12, 15, 15, 14], [0.1, 13, 7, 15.9, 15, 11],
         [0.1, 9, 8, 15.9, 13, 12], [0.1, 5, 9, 15.9, 9, 13], [0.1, 1, 10, 15.9, 5, 14],
         [0.1, -1, 11, 15.9, 1, 15]]
_PERPENDICULAR = [[2, -2, 14, 14, 14, 18], [1, 8, 12, 15, 15, 14], [0.1, 13, 7, 15.9, 15, 11],
                  [0.1, 9, 8, 15.9, 13, 12], [0.1, 5, 9, 15.9, 9, 13], [0.1, 1, 10, 15.9, 5, 14],
                  [0.1, -3, 11, 15.9, 1, 15]]
_RETRACTED = [[2, -2, 14, 14, 14, 18], [0, -5, 8, 16, 16, 14]]
_EXTENDED = [[2, -2, 14, 14, 14, 18], [3, -4, 10, 13, 13, 14], [2, -4, 6, 14, 14, 10],
             [0, -5, 0, 16, 16, 6]]
# `FUNNEL_COLLISION = shape(0, 0, 0, 16, 4, 16).forDirectional(UP)`: a four texel slab on the
# side the funnel is against.
_SLAB = {"up": [[0, 0, 0, 16, 4, 16]], "down": [[0, 12, 0, 16, 16, 16]],
         "north": [[0, 0, 12, 16, 16, 16]], "south": [[0, 0, 0, 16, 16, 4]],
         "west": [[12, 0, 0, 16, 16, 16]], "east": [[0, 0, 0, 4, 16, 16]]}


def _slab(facing):
    return [[v / 16 for v in box] for box in _SLAB[facing]]


def _funnel_shape(facing):
    """`FunnelBlock.getShape`: a floor, a ceiling or a wall funnel."""
    if facing == "up":
        return _clamped(_FLOOR)
    if facing == "down":
        return _clamped(_CEILING)
    return _facing(_WALL, facing)


_SHAPE = {"properties": ["facing"], "values": {f: _funnel_shape(f) for f in FACINGS}}
# `FunnelBlock.getCollisionShape`: an item meets the slab of a wall funnel, not its body.
_ITEM = {"properties": ["facing"], "values": {
    f: (_slab(f) if f in HORIZONTALS else _funnel_shape(f)) for f in FACINGS}}


def _belt_shape(facing, shape):
    """`BeltFunnelBlock.getShape`."""
    boxes = {"retracted": _RETRACTED, "extended": _EXTENDED}.get(shape, _PERPENDICULAR)
    return _facing(boxes, facing)


_BELT_SHAPE = {"properties": ["facing", "shape"], "values": {
    f"{f},{s}": _belt_shape(f, s) for f in HORIZONTALS for s in SHAPES}}
# `BeltFunnelBlock.getCollisionShape`: only a pushing or pulling funnel has the slab for items.
_BELT_ITEM = {"properties": ["facing", "shape"], "values": {
    f"{f},{s}": _slab(f) if s in ("pushing", "pulling") else _belt_shape(f, s)
    for f in HORIZONTALS for s in SHAPES}}


def _funnel(base, color):
    declared = {**FACING_STATE, "powered": {"type": "boolean"}, "extracting": {"type": "boolean"}}
    value = block(declared, {"facing": "north", "powered": "false", "extracting": "false"},
                  base, color, _SHAPE, render_shape="MODEL")
    value["collision_context_shapes"] = {"item": _ITEM}
    return value


def _belt_funnel(base, color):
    declared = {**HORIZONTAL_FACING_STATE, "powered": {"type": "boolean"},
                "shape": {"type": "enum", "values": SHAPES}}
    value = block(declared, {"facing": "north", "powered": "false", "shape": "retracted"},
                  base, color, _BELT_SHAPE, render_shape="MODEL")
    value["collision_context_shapes"] = {"item": _BELT_ITEM}
    value["item"] = False  # a funnel placed on a belt is the funnel's own item
    return value


BLOCKS = {
    "andesite_funnel": _funnel(ANDESITE, STONE),
    "andesite_belt_funnel": _belt_funnel(ANDESITE, STONE),
    "brass_funnel": _funnel(SOFT_METAL, TERRACOTTA_YELLOW),
    "brass_belt_funnel": _belt_funnel(SOFT_METAL, TERRACOTTA_YELLOW),
}
PICKAXE_ONLY = set(BLOCKS)
TAB = ["andesite_funnel", "brass_funnel"]

# What the funnel's block entity draws besides the block's model (`AllPartialModels.FUNNEL_FLAP`,
# `BELT_FUNNEL_FLAP`).
MODELS = ["block/funnel/flap", "block/belt_funnel/flap"]

NAMES = {
    "block.create.andesite_funnel": "Andesite Funnel",
    "block.create.andesite_belt_funnel": "Andesite Belt Funnel",
    "block.create.brass_funnel": "Brass Funnel",
    "block.create.brass_belt_funnel": "Brass Belt Funnel",
}

# `.loot((p, b) -> p.dropOther(b, ANDESITE_FUNNEL.get()))`, and the brass one's.
LOOT = {name: [{
    "rolls": 1,
    "entries": [{"type": "minecraft:item", "name": f"create:{name.replace('_belt', '')}"}],
    "condition": {"type": "minecraft:survives_explosion"},
}] for name in ("andesite_belt_funnel", "brass_belt_funnel")}

# `CreateStandardRecipeGen` (`crafting/logistics/andesite_funnel`, `brass_funnel`) in Horizon's
# 26.3 form.
RECIPES = {
    "crafting/logistics/andesite_funnel": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["A", "K"],
        "key": {"A": "create:andesite_alloy", "K": "minecraft:dried_kelp"},
        "result": {"id": "create:andesite_funnel", "count": 2},
    },
    "crafting/logistics/brass_funnel": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["E", "A", "K"],
        "key": {"A": "#c:ingots/brass", "E": "create:electron_tube", "K": "minecraft:dried_kelp"},
        "result": {"id": "create:brass_funnel", "count": 2},
    },
}
