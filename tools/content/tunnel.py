"""The Andesite and Brass Tunnels: `create:andesite_tunnel`, `create:brass_tunnel`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ANDESITE_TUNNEL` and `BRASS_TUNNEL`
(`BuilderTransformers.beltTunnel`: `SharedProperties.stone`, `noOcclusion`, `pickaxeOnly`;
`MapColor.STONE` and `MapColor.TERRACOTTA_YELLOW`), `BeltTunnelBlock` (`axis` is
`HORIZONTAL_AXIS`, `shape` is `BeltTunnelBlock.Shape`), `BeltTunnelShapes`,
`recipe/crafting/logistics/andesite_tunnel` and `brass_tunnel`. The blocks are in the chunk mesh
(Create's `block/tunnel/<type>_tunnel/<shape>` models, picked by the imported blockstates); the
block entity draws the flaps. The behaviour is `native/api/src/machines/tunnel.rs`.

Shapes are cut to the block's own cell (Create's reach 5 texels below it, into the belt).
"""

from gen_content import ANDESITE, block

STONE = 11  # `MapColor.STONE`
TERRACOTTA_YELLOW = 40  # `MapColor.TERRACOTTA_YELLOW`

SHAPE_NAMES = ["straight", "window", "closed", "t_left", "t_right", "cross"]  # `Shape`
AXES = ["x", "z"]

# `BeltTunnelShapes`, in texels, cut at the bottom of the block: the whole block less the
# opening of each way out (`box(2, -5, 14, 14, 10, 16)` turned to face each side).
_BLOCK = [0, 0, 0, 16, 16, 16]
_OPENINGS = {
    "south": [2, 0, 14, 14, 10, 16],
    "north": [2, 0, 0, 14, 10, 2],
    "east": [14, 0, 2, 16, 10, 14],
    "west": [0, 0, 2, 2, 10, 14],
}
_HORIZONTALS = ["north", "south", "west", "east"]


def _minus(openings):
    """The block less the boxes `openings`, as boxes in blocks: cells of the grid the openings
    cut it into that no opening covers, with neighbours joined back together."""
    holes = [_OPENINGS[o] for o in openings]
    cuts = [sorted({0, 16, *(h[i] for h in holes), *(h[i + 3] for h in holes)}) for i in range(3)]
    boxes = []
    for x0, x1 in zip(cuts[0], cuts[0][1:]):
        for y0, y1 in zip(cuts[1], cuts[1][1:]):
            for z0, z1 in zip(cuts[2], cuts[2][1:]):
                inside = any(h[0] <= x0 and x1 <= h[3] and h[1] <= y0 and y1 <= h[4]
                             and h[2] <= z0 and z1 <= h[5] for h in holes)
                if not inside:
                    boxes.append([x0, y0, z0, x1, y1, z1])
    merged = True
    while merged:
        merged = False
        for i, a in enumerate(boxes):
            for j, b in enumerate(boxes):
                if i >= j:
                    continue
                for axis in range(3):
                    others = [k for k in range(3) if k != axis]
                    if all(a[k] == b[k] and a[k + 3] == b[k + 3] for k in others) \
                            and a[axis + 3] == b[axis]:
                        boxes[i] = a[:axis + 3] + [b[axis + 3]] + a[axis + 4:]
                        del boxes[j]
                        merged = True
                        break
                if merged:
                    break
            if merged:
                break
    return [[round(v / 16, 6) for v in box] for box in boxes]


def _open_sides(axis, shape):
    """The sides the shape has openings on: both ends of the axis, and across it the side or
    sides the T or cross opens to (`BeltTunnelShapes.getShape`)."""
    ends = ["north", "south"] if axis == "z" else ["west", "east"]
    # `left` is the positive end of the axis turned clockwise: west along z, south along x.
    left = "west" if axis == "z" else "south"
    right = {"west": "east", "south": "north"}[left]
    if shape == "t_left":
        return ends + [left]
    if shape == "t_right":
        return ends + [right]
    if shape == "cross":
        return list(_HORIZONTALS)
    return ends


_SHAPE = {"properties": ["axis", "shape"], "values": {
    f"{axis},{shape}": _minus(_open_sides(axis, shape)) for axis in AXES for shape in SHAPE_NAMES}}


def _tunnel(color):
    declared = {"axis": {"type": "enum", "values": AXES},
                "shape": {"type": "enum", "values": SHAPE_NAMES}}
    return block(declared, {"axis": "x", "shape": "straight"}, ANDESITE, color, _SHAPE,
                 render_shape="MODEL")


BLOCKS = {
    "andesite_tunnel": _tunnel(STONE),
    "brass_tunnel": _tunnel(TERRACOTTA_YELLOW),
}
PICKAXE_ONLY = set(BLOCKS)
TAB = ["andesite_tunnel", "brass_tunnel"]

# What the tunnel's block entity draws besides the block's model (`AllPartialModels.BELT_TUNNEL_FLAP`).
MODELS = ["block/belt_tunnel/flap"]

NAMES = {
    "block.create.andesite_tunnel": "Andesite Tunnel",
    "block.create.brass_tunnel": "Brass Tunnel",
}

# `CreateStandardRecipeGen` (`crafting/logistics/andesite_tunnel`, `brass_tunnel`) in Horizon's
# 26.3 form.
RECIPES = {
    "crafting/logistics/andesite_tunnel": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["AA", "KK"],
        "key": {"A": "create:andesite_alloy", "K": "minecraft:dried_kelp"},
        "result": {"id": "create:andesite_tunnel", "count": 2},
    },
    "crafting/logistics/brass_tunnel": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["E ", "AA", "KK"],
        "key": {"A": "#c:ingots/brass", "E": "create:electron_tube", "K": "minecraft:dried_kelp"},
        "result": {"id": "create:brass_tunnel", "count": 2},
    },
}
