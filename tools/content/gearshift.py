"""Declarations for the Gearshift (module `gearshift`).

Values from Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.GEARSHIFT` (stone, no occlusion, PODZOL,
no stress impact), `data/create/recipe/crafting/kinetics/gearshift.json`.
"""
from gen_content import ANDESITE, AXIS_STATE, PODZOL, block

BLOCKS = {
    "gearshift": block({**AXIS_STATE, "powered": {"type": "boolean"}},
                       {"axis": "y", "powered": "false"}, ANDESITE, PODZOL, render_shape="MODEL"),
}

NAMES = {"block.create.gearshift": "Gearshift"}

RECIPES = {
    # `crafting/kinetics/gearshift`.
    "gearshift": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:andesite_casing", "create:cogwheel", "#c:dusts/redstone"],
        "result": {"id": "create:gearshift"},
    },
}

TAGS = {"c:item/dusts/redstone": ["minecraft:redstone"]}

TAB = ["gearshift"]
