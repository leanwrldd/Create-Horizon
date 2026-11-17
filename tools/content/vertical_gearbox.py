"""Vertical Gearbox (module `vertical_gearbox`).

Create `mc1.21.1/dev` at 4148fd8: `VerticalGearboxItem` (a `BlockItem` for the Gearbox that turns
the placed block onto its side), `recipe/crafting/kinetics/vertical_gearbox` and
`vertical_gearbox_from_conversion`, plus the Gearbox's own `gearbox_from_conversion`. The item
model is Create's `item/vertical_gearbox`, read from the jar; the behaviour is
`native/api/src/machines/vertical_gearbox.rs`.
"""

ITEMS = {"vertical_gearbox": {"template": "minecraft:iron_ingot", "max_stack_size": 64,
                              "place_block": "create:gearbox"}}

NAMES = {"item.create.vertical_gearbox": "Vertical Gearbox"}

# `AllBlocks.GEARBOX` is registered with `.item(VerticalGearboxItem)` right after the Gearbox.
TAB = ["vertical_gearbox"]

RECIPES = {
    "crafting/kinetics/vertical_gearbox": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["C C", " B ", "C C"],
        "key": {"B": "create:andesite_casing", "C": "create:cogwheel"},
        "result": {"id": "create:vertical_gearbox"},
    },
    "crafting/kinetics/vertical_gearbox_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:gearbox"],
        "result": {"id": "create:vertical_gearbox"},
    },
    "crafting/kinetics/gearbox_from_conversion": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:vertical_gearbox"],
        "result": {"id": "create:gearbox"},
    },
}
