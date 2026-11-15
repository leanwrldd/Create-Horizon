"""Crushing Wheels: `create:crushing_wheel`, `create:crushing_wheel_controller` and every
`create:crushing` recipe of Create's datagen.

Create: `AllBlocks.CRUSHING_WHEEL` (`SharedProperties.stone`, `MapColor.METAL`, `noOcclusion`,
`pickaxeOnly`), `AllBlocks.CRUSHING_WHEEL_CONTROLLER` (`MapColor.STONE`, `noOcclusion`,
`noLootTable`, `air`, `noCollission`, `pushReaction(BLOCK)`), `AllShapes.CRUSHING_WHEEL_COLLISION_SHAPE`,
`AllShapes.CRUSHING_WHEEL_CONTROLLER_COLLISION`, `recipe/mechanical_crafting/crushing_wheel`,
`recipe/crushing/*`, `AllDamageTypes.CRUSH` (`data/create/damage_type/crush.json`).
"""

import importlib.util
from pathlib import Path

from gen_content import (ANDESITE, AXIS_STATE, FACING_STATE, FACINGS, METAL, block,
                         facing_shapes)


def _data(name, const):
    """A generated recipe table beside this file (the plugin loader skips `_` files)."""
    path = Path(__file__).with_name(name)
    spec = importlib.util.spec_from_file_location(f"create_content_{path.stem}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return getattr(module, const)


STONE = 11  # MapColor.STONE
# `shape(0, 0, 0, 16, 13, 16).forDirectional(Direction.DOWN)`: 13 texels deep on the side the
# controller faces. facing_shapes takes a box written for up, so the same box is [3, 16] in y.
CONTROLLER_FLOOR = facing_shapes([[0, 3 / 16, 0, 1, 1, 1]])

# `Properties.of()`: no hardness, no tool, stone sounds. The model is `minecraft:block/air`
# (Create's blockstate); the block entity draws nothing either.
CONTROLLER = block(
    {**FACING_STATE, "valid": {"type": "boolean"}}, {"facing": "down", "valid": "false"},
    {"hardness": 0.0, "explosion_resistance": 0.0, "requires_tool": False,
     "sounds": "minecraft:stone"},
    STONE, render_shape="INVISIBLE")
CONTROLLER["item"] = False  # No item of its own: the wheels place it.
CONTROLLER["collision_shapes"] = CONTROLLER_FLOOR

BLOCKS = {
    # `CRUSHING_WHEEL_COLLISION_SHAPE` is the full cube; the model (`crushing_wheel.obj`) is drawn
    # by the block entity and reaches half a block past the block on its two wide sides.
    "crushing_wheel": block(AXIS_STATE, {"axis": "y"}, ANDESITE, METAL),
    "crushing_wheel_controller": CONTROLLER,
}
# Create's file has one variant per `valid`; every combination of the two properties is spelt out.
BLOCKSTATES = {
    "crushing_wheel_controller": {"variants": {
        f"facing={facing},valid={valid}": {"model": "minecraft:block/air"}
        for facing in FACINGS for valid in ("false", "true")}},
}
PICKAXE_ONLY = {"crushing_wheel"}
MINEABLE = {"crushing_wheel_controller": []}
LOOT = {"crushing_wheel_controller": None}
NAMES = {
    "block.create.crushing_wheel": "Crushing Wheel",
    "block.create.crushing_wheel_controller": "Crushing Wheel Controller",
    "death.attack.create.crush": "%1$s was processed by Crushing Wheels",
    "death.attack.create.crush.player": "%1$s was thrown into Crushing Wheels by %2$s",
}
TAB = ["crushing_wheel"]
MODELS = ["block/crushing_wheel/block"]  # `AllPartialModels.CRUSHING_WHEEL`

RECIPES = {
    **_data("_crushing_wheel_recipes.py", "CRUSHING"),
    # Create's mechanical crafting recipe, as its datagen writes it: it takes the Mechanical
    # Crafter, which this mod doesn't have yet, so the wheel can't be crafted until it does.
    "mechanical_crafting/crushing_wheel": {
        "type": "create:mechanical_crafting",
        "accept_mirrored": False,
        "category": "misc",
        "key": {"A": {"item": "create:andesite_alloy"}, "P": {"tag": "minecraft:planks"},
                "S": {"tag": "c:stones"}},
        "pattern": [" AAA ", "AAPAA", "APSPA", "AAPAA", " AAA "],
        "result": {"count": 2, "id": "create:crushing_wheel"},
        "show_notification": False,
    },
}
# `data/create/damage_type/crush.json`, as Create's datagen writes it.
FILES = {
    "data/create/damage_type/crush.json": {
        "exhaustion": 0.0, "message_id": "create.crush", "scaling": "always"},
}
# The convention tags Create's crushing recipes name, with what NeoForge puts in them.
TAGS = {
    "c:item/stones": ["minecraft:stone", "minecraft:granite", "minecraft:diorite",
                      "minecraft:andesite", "minecraft:deepslate", "minecraft:tuff"],
    "c:item/raw_materials/copper": ["minecraft:raw_copper"],
    "c:item/raw_materials/gold": ["minecraft:raw_gold"],
    "c:item/raw_materials/iron": ["minecraft:raw_iron"],
    "c:item/raw_materials/zinc": ["create:raw_zinc"],
    "c:item/storage_blocks/raw_copper": ["minecraft:raw_copper_block"],
    "c:item/storage_blocks/raw_gold": ["minecraft:raw_gold_block"],
    "c:item/storage_blocks/raw_iron": ["minecraft:raw_iron_block"],
    "c:item/storage_blocks/raw_zinc": ["create:raw_zinc_block"],
}
