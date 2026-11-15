"""Declarations for the Encased Chain Drive and the Adjustable Chain Gearshift (module
`chain_drive`).

Values from Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ENCASED_CHAIN_DRIVE` (stone, no occlusion,
PODZOL) and `ADJUSTABLE_CHAIN_GEARSHIFT` (the same, NETHER), `ChainDriveBlock` (`part`,
`axis_along_first`), `ChainGearshiftBlock` (`powered`), and the recipes under
`data/create/recipe/crafting/kinetics/`.
"""
from gen_content import ANDESITE, AXIS_STATE, PODZOL, block

# `MapColor.NETHER`.
NETHER = 35

DRIVE_STATES = {
    **AXIS_STATE,
    "part": {"type": "enum", "values": ["start", "middle", "end", "none"]},
    "axis_along_first": {"type": "boolean"},
}
DRIVE_DEFAULT = {"axis": "y", "part": "none", "axis_along_first": "false"}

BLOCKS = {
    "encased_chain_drive": block(DRIVE_STATES, DRIVE_DEFAULT, ANDESITE, PODZOL,
                                 render_shape="MODEL"),
    "adjustable_chain_gearshift": block(
        {**DRIVE_STATES, "powered": {"type": "boolean"}}, {**DRIVE_DEFAULT, "powered": "false"},
        ANDESITE, NETHER, render_shape="MODEL"),
}

NAMES = {
    "block.create.encased_chain_drive": "Encased Chain Drive",
    "block.create.adjustable_chain_gearshift": "Adjustable Chain Gearshift",
}

RECIPES = {
    # `crafting/kinetics/encased_chain_drive` and `_from_zinc`.
    "encased_chain_drive": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:andesite_casing", "#c:nuggets/iron", "#c:nuggets/iron",
                        "#c:nuggets/iron"],
        "result": {"id": "create:encased_chain_drive"},
    },
    "encased_chain_drive_from_zinc": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:andesite_casing", "#c:nuggets/zinc", "#c:nuggets/zinc",
                        "#c:nuggets/zinc"],
        "result": {"id": "create:encased_chain_drive"},
    },
    # `crafting/kinetics/adjustable_chain_gearshift`.
    "adjustable_chain_gearshift": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:encased_chain_drive", "create:electron_tube"],
        "result": {"id": "create:adjustable_chain_gearshift"},
    },
}

TAGS = {"c:item/nuggets/iron": ["minecraft:iron_nugget"]}

TAB = ["encased_chain_drive", "adjustable_chain_gearshift"]
