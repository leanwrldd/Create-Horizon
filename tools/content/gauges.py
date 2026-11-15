"""Declarations for the Speedometer and the Stressometer (module `gauges`).

Values from Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.SPEEDOMETER` and `STRESSOMETER`
(`SharedProperties.wooden`, PODZOL, no stress impact), `GaugeBlock` (`facing`, `axis_along_first`),
`AllShapes.GAUGE_SHAPE_UP` and `GaugeShaper`, `data/create/recipe/crafting/kinetics/`. The base
sits in the chunk mesh (Create's blockstate files); the block entity draws the shaft, the heads
and the dials.
"""
from gen_content import AXES, FACING_STATE, FACINGS, PODZOL, WOODEN, block

STATES = {**FACING_STATE, "axis_along_first": {"type": "boolean"}}

# `AllShapes.GAUGE_SHAPE_UP`: shape(1, 0, 0, 15, 2, 16).add(2, 2, 1, 14, 14, 15), in texels.
GAUGE_UP = [[1, 0, 0, 15, 2, 16], [2, 2, 1, 14, 14, 15]]

# How Create's blockstate files turn the model for each state (`blockstates/speedometer.json`):
# (facing, axis_along_first) -> (x turn, y turn). `base` models face up, `base_wall` models
# stand on the west side and look east; the same two boxes of the shape, written for each.
TURNS = {
    ("down", False): (180, 0), ("east", False): (0, 0), ("north", False): (90, 270),
    ("south", False): (90, 90), ("up", False): (0, 0), ("west", False): (0, 180),
    ("down", True): (180, 90), ("east", True): (90, 0), ("north", True): (0, 270),
    ("south", True): (0, 90), ("up", True): (0, 90), ("west", True): (90, 180),
}
# `base_wall`'s two boxes: `GAUGE_UP` with x and y swapped.
GAUGE_WALL = [[0, 1, 0, 2, 15, 16], [2, 2, 1, 14, 14, 15]]


def _turn(box, x_turn, y_turn):
    def point(p):
        x, y, z = (v / 16 - 0.5 for v in p)
        for _ in range(x_turn // 90):
            y, z = z, -y          # x 90: up becomes north
        for _ in range(y_turn // 90):
            x, z = -z, x          # y 90: north becomes east (clockwise from above)
        return [x + 0.5, y + 0.5, z + 0.5]

    a, b = point(box[:3]), point(box[3:])
    return [round(min(a[i], b[i]), 6) for i in range(3)] + [round(max(a[i], b[i]), 6) for i in range(3)]


def _shapes():
    values = {}
    for (facing, along), (x_turn, y_turn) in TURNS.items():
        boxes = GAUGE_UP if facing in ("up", "down") else GAUGE_WALL
        values[f"{facing},{'true' if along else 'false'}"] = [
            _turn(box, x_turn, y_turn) for box in boxes]
    return {"properties": ["facing", "axis_along_first"], "values": values,
            "default": values["north,false"]}


def gauge():
    return block(STATES, {"facing": "north", "axis_along_first": "false"}, WOODEN, PODZOL,
                 _shapes(), render_shape="MODEL")


BLOCKS = {"speedometer": gauge(), "stressometer": gauge()}

# `AllPartialModels.GAUGE_DIAL`, `GAUGE_HEAD_SPEED` and `GAUGE_HEAD_STRESS`.
MODELS = ["block/gauge/dial", "block/gauge/speedometer/head", "block/gauge/stressometer/head"]

NAMES = {
    "block.create.speedometer": "Speedometer",
    "block.create.stressometer": "Stressometer",
}

RECIPES = {
    # `crafting/kinetics/speedometer`.
    "speedometer": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["C", "A"],
        "key": {"A": "create:andesite_casing", "C": "minecraft:compass"},
        "result": {"id": "create:speedometer"},
    },
    # `speedometer_from_conversion` and `stressometer_from_conversion` (`GAUGE_CYCLE`): the only
    # way to a Stressometer.
    "speedometer_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:stressometer"],
        "result": {"id": "create:speedometer"},
    },
    "stressometer_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:speedometer"],
        "result": {"id": "create:stressometer"},
    },
}

TAB = ["speedometer", "stressometer"]
