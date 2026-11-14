"""Generate the Horizon declarations and item definitions for Create."""

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "content"
ASSETS = ROOT / "assets" / "create"
DATA = ROOT / "data" / "create"

AXES = ["x", "y", "z"]
FACINGS = ["north", "south", "east", "west", "up", "down"]


def write(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8", newline="\n")


# --- shapes, in blocks, written for the y axis and turned for the others -----------------

def turn(box, axis):
    """A y-axis box [x0, y0, z0, x1, y1, z1] turned to lie along `axis`."""
    x0, y0, z0, x1, y1, z1 = box
    if axis == "y":
        return box
    if axis == "x":
        return [y0, x0, z0, y1, x1, z1]
    return [x0, z0, y0, x1, z1, y1]


def axis_shapes(boxes):
    return {"property": "axis", "values": {a: [turn(b, a) for b in boxes] for a in AXES}}


SHAFT = [0.375, 0, 0.375, 0.625, 1, 0.625]
SHAPES = {
    "shaft": [SHAFT],
    "cogwheel": [SHAFT, [0.125, 0.375, 0.125, 0.875, 0.625, 0.875]],
    "large_cogwheel": [SHAFT, [0, 0.375, 0, 1, 0.625, 1]],
}


def motor_shape(facing):
    """The motor's body: 10 texels square, 14 long, its back against the face behind it."""
    lo, hi = 0.1875, 0.8125
    along = {"up": (1, 1), "down": (1, -1), "south": (2, 1), "north": (2, -1),
             "east": (0, 1), "west": (0, -1)}[facing]
    box = [lo, lo, lo, hi, hi, hi]
    axis, sign = along
    box[axis], box[axis + 3] = (0, 0.875) if sign > 0 else (0.125, 1)
    return box


def facing_shapes(boxes):
    """Boxes written for facing up, turned so up points along each facing, as Catnip's
    `VoxelShaper.forDirectional` does (every box here is square across, so a turn only moves
    its height onto the facing's axis)."""
    def turned(box, facing):
        x0, y0, z0, x1, y1, z1 = box
        axis, sign = {"up": (1, 1), "down": (1, -1), "south": (2, 1), "north": (2, -1),
                      "east": (0, 1), "west": (0, -1)}[facing]
        lo, hi = (y0, y1) if sign > 0 else (1 - y1, 1 - y0)
        out = [x0, x0, x0, x1, x1, x1]
        out[axis], out[axis + 3] = lo, hi
        return out
    return {"property": "facing", "values": {f: [turned(b, f) for b in boxes] for f in FACINGS}}


# Create's `AllShapes.CRANK`: shape(5, 0, 5, 11, 6, 11).add(1, 3, 1, 15, 8, 15), in texels.
CRANK = [[5 / 16, 0, 5 / 16, 11 / 16, 6 / 16, 11 / 16],
         [1 / 16, 3 / 16, 1 / 16, 15 / 16, 8 / 16, 15 / 16]]

# What each block copies in Create (`AllBlocks`, `SharedProperties`), with vanilla's numbers for
# those blocks as Horizon's prepared game data has them.
ANDESITE = {"hardness": 1.5, "explosion_resistance": 6.0, "requires_tool": True,
            "sounds": "minecraft:andesite"}  # SharedProperties.stone
WOODEN = {"hardness": 2.0, "explosion_resistance": 2.0, "requires_tool": False,
          "sounds": "minecraft:stripped_spruce_wood"}  # SharedProperties.wooden
WOOD_SOUND = "minecraft:oak_planks"  # SoundType.WOOD
# MapColor ids.
METAL, DIRT, GRAY, PURPLE, PODZOL = 6, 10, 21, 24, 34


def block(extra_states, default, base, map_color, shapes=None, sounds=None,
          render_shape="ENTITYBLOCK_ANIMATED"):
    properties = dict(base, map_color=map_color, render_shape=render_shape)
    if sounds:
        properties["sounds"] = sounds
    value = {
        "template": "minecraft:glass",
        "item": True,
        "properties": properties,
    }
    if extra_states:
        value["state_properties"] = extra_states
        value["default_state"] = default
    if shapes:
        value["shapes"] = shapes
        value["collision_shapes"] = shapes
    return value


AXIS_STATE = {"axis": {"type": "enum", "values": AXES}}
FACING_STATE = {"facing": {"type": "enum", "values": FACINGS}}
BLOCKS = {
    "shaft": block(AXIS_STATE, {"axis": "y"}, ANDESITE, METAL, axis_shapes(SHAPES["shaft"])),
    "cogwheel": block(AXIS_STATE, {"axis": "y"}, ANDESITE, DIRT,
                      axis_shapes(SHAPES["cogwheel"]), sounds=WOOD_SOUND),
    "large_cogwheel": block(AXIS_STATE, {"axis": "y"}, ANDESITE, DIRT,
                            axis_shapes(SHAPES["large_cogwheel"]), sounds=WOOD_SOUND),
    "gearbox": block(AXIS_STATE, {"axis": "y"}, ANDESITE, PODZOL),
    "creative_motor": block(
        FACING_STATE, {"facing": "north"}, ANDESITE, PURPLE,
        {"property": "facing", "values": {f: [motor_shape(f)] for f in FACINGS}},
    ),
    "hand_crank": block(FACING_STATE, {"facing": "up"}, WOODEN, PODZOL, facing_shapes(CRANK)),
    # Create gives the wheel no shape of its own: a full block.
    "water_wheel": block(AXIS_STATE, {"axis": "z"}, WOODEN, DIRT),
    # `AllShapes.CASING_13PX` facing up. Its model sits in the chunk mesh; the block entity
    # draws only the stack on top. No waterlogging yet, as for the other blocks.
    "depot": block({}, {}, ANDESITE, GRAY, {"default": [[0, 0, 0, 1, 13 / 16, 1]]},
                   render_shape="MODEL"),
}
# `pickaxeOnly()` in Create; every other block here is `axeOrPickaxe()`.
PICKAXE_ONLY = {"shaft", "creative_motor"}
ITEMS = {
    "andesite_alloy": {"template": "minecraft:iron_ingot", "max_stack_size": 64},
    "wrench": {
        "template": "minecraft:iron_ingot",
        "max_stack_size": 1,
        # Horizon checks the tool before rolling drops for a player's break.
        "components": {"minecraft:tool": {
            "rules": [{
                "blocks": [f"create:{name}" for name, block_data in BLOCKS.items()
                           if block_data["properties"]["requires_tool"]],
                "correct_for_drops": True,
            }],
            "damage_per_block": 0,
        }},
    },
}
NAMES = {
    "block.create.shaft": "Shaft",
    "block.create.cogwheel": "Cogwheel",
    "block.create.large_cogwheel": "Large Cogwheel",
    "block.create.gearbox": "Gearbox",
    "block.create.creative_motor": "Creative Motor",
    "block.create.hand_crank": "Hand Crank",
    "block.create.water_wheel": "Water Wheel",
    "block.create.depot": "Depot",
    "item.create.andesite_alloy": "Andesite Alloy",
    "item.create.wrench": "Wrench",
    "itemGroup.create.base": "Create",
}


RECIPES = {
    "andesite_alloy": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["AN", "NA"],
        "key": {"A": "minecraft:andesite", "N": "minecraft:iron_nugget"},
        "result": {"id": "create:andesite_alloy"},
    },
    "shaft": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["A", "A"], "key": {"A": "create:andesite_alloy"},
        "result": {"id": "create:shaft", "count": 8},
    },
    "cogwheel": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:shaft", "#minecraft:planks"],
        "result": {"id": "create:cogwheel"},
    },
    "large_cogwheel": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:shaft", "#minecraft:planks", "#minecraft:planks"],
        "result": {"id": "create:large_cogwheel"},
    },
    # Create puts an Andesite Casing in the middle; this slice has no casing yet.
    "gearbox": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" C ", "CAC", " C "],
        "key": {"C": "create:cogwheel", "A": "create:andesite_alloy"},
        "result": {"id": "create:gearbox"},
    },
    "wrench": {
        "type": "minecraft:crafting_shaped", "category": "equipment",
        "pattern": ["GG", "GC", " S"],
        "key": {"G": "minecraft:gold_ingot", "C": "create:cogwheel", "S": "minecraft:stick"},
        "result": {"id": "create:wrench"},
    },
    "hand_crank": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["PPP", "  A"],
        "key": {"P": "#minecraft:planks", "A": "create:andesite_alloy"},
        "result": {"id": "create:hand_crank"},
    },
    "water_wheel": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["PPP", "PSP", "PPP"],
        "key": {"P": "#minecraft:planks", "S": "create:shaft"},
        "result": {"id": "create:water_wheel"},
    },
}


def item_model(name):
    model = {"type": "minecraft:model", "model": f"create:item/{name}"}
    if name == "wrench":
        return {"type": "minecraft:composite", "models": [
            model, {"type": "minecraft:model", "model": "create:item/wrench/gear"},
        ]}
    return model


def main():
    for name, value in BLOCKS.items():
        write(DATA / "horizon" / "block" / f"{name}.json", value)
        write(DATA / "loot_table" / "blocks" / f"{name}.json", {
            "type": "minecraft:block",
            "pools": [{
                "rolls": 1,
                "entries": [{"type": "minecraft:item", "name": f"create:{name}"}],
                "condition": {"type": "minecraft:survives_explosion"},
            }],
            "random_sequence": f"create:blocks/{name}",
        })
    for name, value in ITEMS.items():
        write(DATA / "horizon" / "item" / f"{name}.json", value)
    for name in [*BLOCKS, *ITEMS]:
        write(ASSETS / "items" / f"{name}.json", {"model": item_model(name)})
    for name, value in RECIPES.items():
        write(DATA / "recipe" / f"{name}.json", value)
    write(DATA / "horizon" / "creative_tab" / "kinetics.json", {
        "title": "itemGroup.create.base",
        "icon": "create:cogwheel",
        "items": [f"create:{name}" for name in ["creative_motor", "shaft", "cogwheel",
                  "large_cogwheel", "gearbox", "hand_crank", "water_wheel", "depot", "wrench",
                  "andesite_alloy"]],
    })
    write(ROOT / "data" / "minecraft" / "tags" / "block" / "mineable" / "pickaxe.json",
          {"values": [f"create:{name}" for name in BLOCKS]})
    write(ROOT / "data" / "minecraft" / "tags" / "block" / "mineable" / "axe.json",
          {"values": [f"create:{name}" for name in BLOCKS if name not in PICKAXE_ONLY]})
    write(ASSETS / "lang" / "en_us.json", NAMES)


if __name__ == "__main__":
    main()
