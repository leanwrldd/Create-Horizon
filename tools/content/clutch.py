"""Declarations for the Clutch (module `clutch`).

Values from Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.CLUTCH` (stone, no occlusion, PODZOL, no
stress impact), `data/create/recipe/crafting/kinetics/clutch.json`.
"""
from gen_content import ANDESITE, AXIS_STATE, PODZOL, block

BLOCKS = {
    # The casing sits in the chunk mesh; the block entity draws the two shaft halves.
    "clutch": block({**AXIS_STATE, "powered": {"type": "boolean"}},
                    {"axis": "y", "powered": "false"}, ANDESITE, PODZOL, render_shape="MODEL"),
}

NAMES = {"block.create.clutch": "Clutch"}

RECIPES = {
    # `crafting/kinetics/clutch`.
    "clutch": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:andesite_casing", "create:shaft", "#c:dusts/redstone"],
        "result": {"id": "create:clutch"},
    },
}

# NeoForge's redstone dust tag, which the recipe names.
TAGS = {"c:item/dusts/redstone": ["minecraft:redstone"]}

TAB = ["clutch"]
