"""Crushed Raw Osmium and the other compat crushed ores (module `crushed_osmium`; declarations only).

Create `mc1.21.1/dev` at 4148fd8: `AllItems.compatCrushedOre(CommonMetal)` registers
`create:crushed_raw_<metal>` as a `TagDependentIngredientItem` for osmium, platinum, silver, tin,
lead, quicksilver, aluminum (Create's "bauxite"), uranium and nickel, tagged
`create:crushed_raw_materials`. `shouldHide()` is true while the metal's `c:ores/<metal>` tag has no
member, and Create's creative tab then leaves the item out. Nothing in this package gives such a
tag a member, so these items are declared but left out of the creative tab (they are still in
`/give` and in recipes), which is what Create shows without a mod that adds the metal.

The recipes in Create's datagen:
- `crushing/<metal>_ore`, `raw_<metal>` and `raw_<metal>_block` are guarded by
  `neoforge:not(tag_empty(<tag>))`. Horizon has no `neoforge:conditions`; each recipe takes its
  own tag as the ingredient, so with an empty tag it never matches: the same outcome. They are kept
  here (condition dropped) and match as soon as a content mod or data pack fills the tag.
- `splashing/<mod>/crushed_raw_<metal>`, `smelting/` and `blasting/` `*_compat_<mod>` are guarded by
  `neoforge:mod_loaded` and give another mod's nuggets and ingots, which don't exist here: left out.
"""

# `CommonMetal`: the lowercase name used in the item id and in the `c:` tags.
METALS = ("osmium", "platinum", "silver", "tin", "lead", "quicksilver", "aluminum", "uranium",
          "nickel")


def _crushing():
    recipes = {}
    for metal in METALS:
        crushed = f"create:crushed_raw_{metal}"
        xp = {"chance": 0.75, "id": "create:experience_nugget"}
        recipes[f"crushing/{metal}_ore"] = {
            "type": "create:crushing", "processing_time": 400,
            "ingredients": [{"tag": f"c:ores/{metal}"}],
            "results": [{"id": crushed}, {"chance": 0.75, "id": crushed}, dict(xp)],
        }
        recipes[f"crushing/raw_{metal}"] = {
            "type": "create:crushing", "processing_time": 400,
            "ingredients": [{"tag": f"c:raw_materials/{metal}"}],
            "results": [{"id": crushed}, dict(xp)],
        }
        recipes[f"crushing/raw_{metal}_block"] = {
            "type": "create:crushing", "processing_time": 400,
            "ingredients": [{"tag": f"c:storage_blocks/raw_{metal}"}],
            "results": [{"count": 9, "id": crushed}, {**xp, "count": 9}],
        }
    return recipes


ITEMS = {f"crushed_raw_{metal}": {"template": "minecraft:iron_ingot", "max_stack_size": 64}
         for metal in METALS}

NAMES = {f"item.create.crushed_raw_{metal}": f"Crushed Raw {metal.capitalize()}"
         for metal in METALS}

RECIPES = _crushing()

TAGS = {"create:item/crushed_raw_materials": [f"create:crushed_raw_{metal}" for metal in METALS]}
