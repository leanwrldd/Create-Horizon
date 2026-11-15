"""One-off generator for the Mechanical Saw's recipe table (not a plugin: the leading underscore
keeps tools/gen_content.py from loading it). Writes _saw_recipes.py beside it; saw.py imports
that, so the build never needs the Create checkout.

    python tools/content/_saw_recipes_gen.py C:/Users/latif/Claude/Projects/create-java-src [prepared game data dir]

CUTTING: every `create:cutting` recipe of Create's datagen (`recipe/cutting/*.json`), as it writes
them. Those under `compat/` carry `neoforge:conditions` naming another mod and are left out.

STONECUTTING: Create's saw also cuts what a stonecutter does (`SawBlockEntity.getRecipes`:
`allowStonecuttingOnSaw`, on by default, time 50 for a recipe that isn't a `CuttingRecipe`), but
Horizon's `Host` cannot list the game's stonecutting recipes. So the stonecutting recipes are
baked in here, written as `create:cutting` recipes of 50 ticks: vanilla's from the prepared game
data (`data/minecraft/recipe`), and Create's own from its datagen (`recipe/*.json` of type
`minecraft:stonecutting`). A data pack's changes to stonecutting are not seen.
"""

import json
import pprint
import re
import sys
from pathlib import Path

DEFAULT_DATA = Path.home() / "AppData/Roaming/horizon/.horizon-cache/mod-content/5cce03fe38433e65"
DEFAULT_TIME = 50  # SawBlockEntity.start: `int time = 50`


def table(folder):
    out = {}
    for path in sorted(folder.glob("*.json")):  # top level only: compat/ is other mods'
        recipe = json.loads(path.read_text(encoding="utf-8"))
        if recipe.get("neoforge:conditions"):
            continue
        out[f"cutting/{path.stem}"] = recipe
    return out


def stonecutting(folder, prefix):
    out = {}
    for path in sorted(folder.glob("*.json")):
        recipe = json.loads(path.read_text(encoding="utf-8"))
        if recipe.get("type") != "minecraft:stonecutting" or recipe.get("neoforge:conditions"):
            continue
        result = recipe["result"]
        out[f"cutting/stonecutting/{prefix}/{path.stem}"] = {
            "type": "create:cutting",
            "ingredients": [recipe["ingredient"]],
            "processing_time": DEFAULT_TIME,
            "results": [{k: v for k, v in result.items() if k in ("id", "count", "components")}],
        }
    return out


# RuntimeDataGenerator.cuttingRecipes: Create writes these at load time from the item registry,
# so its datagen output has none. Ported for the vanilla items of the prepared game data.
STRIPPED_PREFIX = re.compile(r"(\w*)??stripped_(\w*)(_log|_wood|_stem|_hyphae|_block|(?<!_)wood)()$")
STRIPPED_SUFFIX = re.compile(r"(\w*)(_log|_wood|_stem|_hyphae|_block|(?<!_)wood)(\w*)_stripped(\w*)")
NON_STRIPPED = re.compile(r"^(?!stripped_)([a-z_]+)(_log|_wood|_stem|_hyphae|(?<!bioshroom)_block)(([a-z_]+)(?<!_stripped))?$")
WOOD_TIME = 50  # RuntimeDataGenerator.simpleWoodRecipe: `.duration(50)`


def wood(items):
    """`RuntimeDataGenerator.cuttingRecipes` over `items` (namespaced ids): logs and woods to their
    stripped variant, stripped ones to planks, planks to stairs, slabs, fences and the rest."""
    out = {}

    def recipe(source, target, amount=1, tag=False):
        if target not in items:
            return
        ns, src = source.split(":")
        name = ("tag_" + src.split("/")[-1] if tag else src) + "_to_" + target.split(":")[1]
        result = {"id": target} if amount == 1 else {"id": target, "count": amount}
        out[f"cutting/runtime_generated/compat/{ns}/{name}"] = {
            "type": "create:cutting",
            "ingredients": [{"tag": source} if tag else {"item": source}],
            "processing_time": WOOD_TIME,
            "results": [result],
        }

    for item in sorted(items):
        ns, path = item.split(":")
        match = STRIPPED_PREFIX.search(path)
        prefixed = match is not None
        if match is None:
            match = STRIPPED_SUFFIX.search(path)
        no_stripped = False
        if match is None and f"{ns}:stripped_{path}" not in items and f"{ns}:{path}_stripped" not in items:
            match = NON_STRIPPED.search(path)
            no_stripped = True
        if match is None:
            continue
        g = lambda i: match.group(i) or ""
        prefix = g(1) if prefixed else ""
        suffix = g(3) + g(4) if not prefixed and not no_stripped else ""
        kind = g(3 if prefixed else 2)
        name = g(2 if prefixed else 1)
        base = name + ("wood" if kind == "wood" else "")
        non_stripped = f"{ns}:{prefix}{name}{kind}{suffix}"
        planks = f"{ns}:{base}_planks"
        count = 3 if "block" in kind else 6
        if not no_stripped:
            if non_stripped in items:
                recipe(non_stripped, item)
            recipe(item, planks, count)
        elif planks in items:
            raise SystemExit(f"{item}: a tag recipe (runtime_generated/compat tag) is needed; not ported")
        if "_wood" not in path and "_hyphae" not in path and planks in items:
            for part, amount in (("stairs", 1), ("slab", 2), ("fence", 1), ("fence_gate", 1),
                                 ("door", 1), ("trapdoor", 1), ("pressure_plate", 1),
                                 ("button", 1), ("sign", 1)):
                recipe(planks, f"{ns}:{base}_{part}", amount)
    return out


def main():
    create = Path(sys.argv[1]) / "src" / "generated" / "resources" / "data" / "create" / "recipe"
    data = Path(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_DATA
    cutting = table(create / "cutting")
    stone = {**stonecutting(data / "data" / "minecraft" / "recipe", "minecraft"),
             **stonecutting(create, "create")}
    items = set(json.loads((data / "items.json").read_text(encoding="utf-8")))
    woods = wood(items)
    text = ('"""Create\'s `create:cutting` recipes as its datagen writes them, and the stonecutting '
            'recipes the saw also cuts, written as cutting recipes (generated by '
            '_saw_recipes_gen.py), and the wood recipes Create writes at load time."""\n\n'
            f"CUTTING = {pprint.pformat(cutting, width=100, sort_dicts=True)}\n\n"
            f"STONECUTTING = {pprint.pformat(stone, width=100, sort_dicts=True)}\n\n"
            f"WOOD = {pprint.pformat(woods, width=100, sort_dicts=True)}\n")
    (Path(__file__).resolve().parent / "_saw_recipes.py").write_text(text, encoding="utf-8", newline="\n")
    print(len(cutting), "cutting,", len(stone), "stonecutting,", len(woods), "wood")


if __name__ == "__main__":
    main()
