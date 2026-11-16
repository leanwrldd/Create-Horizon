"""The Windmill Bearing, the Windmill Sail Frame and the Windmill Sails (module `windmill`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.WINDMILL_BEARING` (`PODZOL`, axe or pickaxe,
`BuilderTransformers.bearing("windmill", "gearbox")`, `CStress.setCapacity(512.0)`),
`SAIL_FRAME` / `SAIL` (`white_sail`) / `DYED_SAILS` (`SharedProperties.wooden`, `SoundType
.SCAFFOLDING`, `noOcclusion`, `axeOnly`; the dyed ones have no item and drop a white sail),
`SailBlock` (`facing`), `AllShapes.SAIL` / `SAIL_FRAME` / `SAIL_FRAME_COLLISION`,
`AllPartialModels.BEARING_TOP_WOODEN` and the recipes under `data/create/recipe/crafting/kinetics/`.
The behaviour is `native/api/src/machines/bearing.rs` and `windmill.rs`.
"""

from gen_content import ANDESITE, DIRT, FACING_STATE, PODZOL, WOODEN, block, facing_shapes

WINDMILL_BEARING = block(FACING_STATE, {"facing": "north"}, ANDESITE, PODZOL, render_shape="MODEL")

# `AllShapes`: `SAIL` shape(0, 5, 0, 16, 10, 16), `SAIL_FRAME` shape(0, 5, 0, 16, 9, 16) and the
# frame's collision, the same with (2, 0, 2, 14, 16, 14) erased (a ring), each written facing up.
_SAIL = facing_shapes([[0, 5 / 16, 0, 1, 10 / 16, 1]])
_FRAME = facing_shapes([[0, 5 / 16, 0, 1, 9 / 16, 1]])
_RING = facing_shapes([
    [0, 5 / 16, 0, 1, 9 / 16, 2 / 16], [0, 5 / 16, 14 / 16, 1, 9 / 16, 1],
    [0, 5 / 16, 2 / 16, 2 / 16, 9 / 16, 14 / 16], [14 / 16, 5 / 16, 2 / 16, 1, 9 / 16, 14 / 16],
])

# DyeColor order and each colour's map colour (`DyeColor.getMapColor`).
COLOURS = [
    ("white", 8), ("orange", 15), ("magenta", 16), ("light_blue", 17), ("yellow", 18),
    ("lime", 19), ("pink", 20), ("gray", 21), ("light_gray", 22), ("cyan", 23), ("purple", 24),
    ("blue", 25), ("brown", 26), ("green", 27), ("red", 28), ("black", 29),
]


def _sail(map_color, shapes, collision=None):
    value = block(FACING_STATE, {"facing": "up"}, WOODEN, map_color, shapes,
                  sounds="minecraft:scaffolding", render_shape="MODEL")
    value["properties"]["occludes"] = False  # `noOcclusion()`
    if collision:
        value["collision_shapes"] = collision
    return value


BLOCKS = {"windmill_bearing": WINDMILL_BEARING, "sail_frame": _sail(DIRT, _FRAME, _RING)}
NAMES = {
    "block.create.windmill_bearing": "Windmill Bearing",
    "block.create.sail_frame": "Windmill Sail Frame",
    "block.create.white_sail": "Windmill Sail",
}
MINEABLE = {"sail_frame": ["axe"]}  # `axeOnly()`
LOOT = {}
for colour, map_color in COLOURS:
    name = f"{colour}_sail"
    BLOCKS[name] = _sail(map_color, _SAIL)
    MINEABLE[name] = ["axe"]
    if colour != "white":
        # The dyed sails have no item (`.register()` without `.item()`) and drop a white sail.
        BLOCKS[name]["item"] = False
        LOOT[name] = [{
            "rolls": 1,
            "entries": [{"type": "minecraft:item", "name": "create:white_sail"}],
            "condition": {"type": "minecraft:survives_explosion"},
        }]
        NAMES[f"block.create.{name}"] = f"{colour.replace('_', ' ').title()} Sail"

# What the block entity draws besides the block's model (`AllPartialModels.BEARING_TOP_WOODEN`).
MODELS = ["block/bearing/top_wooden"]

TAB = ["windmill_bearing", "sail_frame", "white_sail"]

# `data/create/tags/block/windmill_sails.json`: the wool and Create's sails.
TAGS = {"create:block/windmill_sails": [f"create:{c}_sail" for c, _ in COLOURS] + ["create:sail_frame"]}

RECIPES = {
    # `crafting/kinetics/windmill_bearing`, `white_sail` and the two conversions, in Horizon's
    # 26.3 form.
    "crafting/kinetics/windmill_bearing": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "#minecraft:wooden_slabs", "C": "#c:stones", "I": "create:shaft"},
        "result": {"id": "create:windmill_bearing", "count": 1},
    },
    "crafting/kinetics/white_sail": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["WS", "SA"],
        "key": {"A": "create:andesite_alloy", "S": "#c:rods/wooden", "W": "#minecraft:wool"},
        "result": {"id": "create:white_sail", "count": 2},
    },
    "crafting/kinetics/white_sail_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:sail_frame"],
        "result": {"id": "create:white_sail", "count": 1},
    },
    "crafting/kinetics/sail_frame_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:white_sail"],
        "result": {"id": "create:sail_frame", "count": 1},
    },
}
