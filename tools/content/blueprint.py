"""The Crafting Blueprint: `create:crafting_blueprint` (a block here, an entity in Create).

Create `mc1.21.1/dev` at 4148fd8: `AllItems.CRAFTING_BLUEPRINT` (`BlueprintItem`, places a
`BlueprintEntity`), `recipe/crafting/appliances/crafting_blueprint`. Horizon has no mod entity type,
so a thin one-block plate with a block entity stands in (see `native/kinetics/src/rules/blueprint.rs`
for the differences). The plate's model is ours, with Create's item texture
(`create:item/crafting_blueprint`) read from the jar; the item model is Create's. The behaviour is
`native/api/src/machines/blueprint.py`'s sibling `blueprint.rs`.
"""
from gen_content import block, facing_shapes

PLATE = facing_shapes([[1 / 16, 0, 1 / 16, 15 / 16, 1 / 16, 15 / 16]])["values"]


def _shapes():
    values = {}
    for facing in ("north", "south", "west", "east"):
        values[f"floor,{facing}"] = PLATE["up"]
        values[f"ceiling,{facing}"] = PLATE["down"]
        values[f"wall,{facing}"] = PLATE[facing]
    return {"properties": ["face", "facing"], "values": values, "default": values["wall,north"]}


# `Properties.of()`-like: a hanging picture breaks at once.
_plain = {"hardness": 0.0, "explosion_resistance": 0.0, "requires_tool": False,
          "sounds": "minecraft:oak_planks"}
_plate = block(
    {"face": {"type": "enum", "values": ["floor", "wall", "ceiling"]},
     "facing": {"type": "enum", "values": ["north", "south", "west", "east"]}},
    {"face": "wall", "facing": "north"}, _plain, 0, _shapes(), render_shape="MODEL")
_plate["collision_shapes"] = {"default": []}

BLOCKS = {"crafting_blueprint": _plate}

# The same turns as the Placard's blockstate file: x for the face, y for the facing.
_TURNS = {"floor": 0, "wall": 90, "ceiling": 180}
_YAW = {"floor": {"north": 0, "east": 90, "south": 180, "west": 270},
        "ceiling": {"north": 180, "east": 270, "south": 0, "west": 90},
        "wall": {"north": 0, "east": 90, "south": 180, "west": 270}}
BLOCKSTATES = {"crafting_blueprint": {"variants": {
    f"face={face},facing={facing}": {"model": "create:block/crafting_blueprint",
                                     **({"x": _TURNS[face]} if _TURNS[face] else {}),
                                     **({"y": _YAW[face][facing]} if _YAW[face][facing] else {})}
    for face in _TURNS for facing in ("north", "south", "west", "east")}}}

FILES = {
    "assets/create/models/block/crafting_blueprint.json": {
        "textures": {"0": "create:item/crafting_blueprint", "particle": "create:item/crafting_blueprint"},
        "elements": [{
            "from": [1, 0, 1], "to": [15, 1, 15],
            "faces": {"up": {"uv": [0, 0, 16, 16], "texture": "#0"},
                      "down": {"uv": [0, 0, 16, 16], "texture": "#0"},
                      "north": {"uv": [0, 0, 16, 1], "texture": "#0"},
                      "south": {"uv": [0, 0, 16, 1], "texture": "#0"},
                      "west": {"uv": [0, 0, 16, 1], "texture": "#0"},
                      "east": {"uv": [0, 0, 16, 1], "texture": "#0"}},
        }],
    },
}

NAMES = {"block.create.crafting_blueprint": "Crafting Blueprint"}

RECIPES = {
    "crafting/appliances/crafting_blueprint": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["minecraft:painting", "minecraft:crafting_table"],
        "result": {"id": "create:crafting_blueprint"},
    },
}

TAB = ["crafting_blueprint"]
