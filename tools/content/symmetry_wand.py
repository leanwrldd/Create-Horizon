"""The Wand of Symmetry (module `symmetry_wand`): `create:wand_of_symmetry`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.WAND_OF_SYMMETRY` (`SymmetryWandItem`: `stacksTo(1)`,
`Rarity.UNCOMMON`, `AssetLookup.itemModelWithPartials`) and its recipe `mechanical_crafting/wand_of_symmetry`
(a `create:mechanical_crafting` recipe, as the Mechanical Crafter reads them). The item model is Create's
`item/wand_of_symmetry` (its bits and core partials, drawn by Create's item renderer, are not drawn here).
Behaviour: `native/api/src/machines/symmetry_wand.rs` and `native/kinetics/src/rules/symmetry_wand*.rs`.
"""

ITEMS = {
    "wand_of_symmetry": {"template": "minecraft:iron_ingot", "max_stack_size": 1, "rarity": "uncommon"},
}

NAMES = {"item.create.wand_of_symmetry": "Wand Of Symmetry"}

RECIPES = {
    # `recipe/mechanical_crafting/wand_of_symmetry`, as Create's datagen writes it.
    "mechanical_crafting/wand_of_symmetry": {
        "type": "create:mechanical_crafting",
        "accept_mirrored": True,
        "category": "misc",
        "key": {"B": {"tag": "c:ingots/brass"}, "E": {"tag": "c:ender_pearls"}, "G": {"tag": "c:glass_blocks"},
                "O": {"tag": "c:obsidians"}, "P": {"item": "create:precision_mechanism"}},
        "pattern": [" G ", "GEG", " P ", " B ", " O "],
        "result": {"count": 1, "id": "create:wand_of_symmetry"},
    },
}

# NeoForge's `c:ender_pearls` and `c:obsidians` (`c:glass_blocks` is declared by `fan.py`).
TAGS = {
    "c:item/ender_pearls": ["minecraft:ender_pearl"],
    "c:item/obsidians": ["minecraft:obsidian"],
}

# `AllCreativeModeTabs`' base tab, after the other gadgets.
TAB = ["wand_of_symmetry"]
