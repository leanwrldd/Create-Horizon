"""Sand Paper and Red Sand Paper (module `sand_paper`).

Create `mc1.21.1/dev` at 4148fd8: `AllItems.SAND_PAPER` and `RED_SAND_PAPER` (`SandPaperItem`:
`durability(8)`, tagged `create:sandpaper`), `recipe/crafting/materials/sand_paper` and
`red_sand_paper`. The `create:sandpaper_polishing` recipe (Rose Quartz) and the sequenced assembly
recipes are written by `deployer.py` from `_deployer_recipes.py`. The behaviour is
`native/api/src/machines/sand_paper.rs`; the item models are Create's `item/sand_paper` and
`item/red_sand_paper`, read from the jar.
"""

# `SandPaperItem` is a damageable item: `Item.Properties.durability(8)` stacks to 1.
ITEMS = {
    name: {
        "template": "minecraft:iron_ingot",
        "max_stack_size": 1,
        "components": {"minecraft:max_damage": 8},
    }
    for name in ("sand_paper", "red_sand_paper")
}

# `AllItems` order: just before the Wrench.
TAB = ["sand_paper", "red_sand_paper"]

NAMES = {
    "item.create.sand_paper": "Sand Paper",
    "item.create.red_sand_paper": "Red Sand Paper",
    "tag.item.create.sandpaper": "Sandpaper",
}

RECIPES = {
    # `recipe/crafting/materials/sand_paper` and `red_sand_paper`, in Horizon's 26.3 form.
    "crafting/materials/sand_paper": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["minecraft:paper", "#c:sands/colorless"],
        "result": {"id": "create:sand_paper"},
    },
    "crafting/materials/red_sand_paper": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["minecraft:paper", "#c:sands/red"],
        "result": {"id": "create:red_sand_paper"},
    },
}

TAGS = {
    # `AllItemTags.SANDPAPER`.
    "create:item/sandpaper": ["create:sand_paper", "create:red_sand_paper"],
    # NeoForge's `c:sands/colorless` and `c:sands/red` (`Tags.Items.SANDS_COLORLESS`, `SANDS_RED`).
    "c:item/sands/colorless": ["minecraft:sand"],
    "c:item/sands/red": ["minecraft:red_sand"],
}
