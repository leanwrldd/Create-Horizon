"""Packages and the Package Filter: the fourteen cardboard boxes (`create:cardboard_package_12x12`,
`..._10x12`, `..._10x8`, `..._12x10` and the ten `create:rare_<name>_package`) and
`create:package_filter`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems` (the loop over `PackageStyles.STYLES` through
`BuilderTransformers.packageItem`: `stacksTo(1)`, tag `create:packages`, lang "Cardboard Package"
or "Rare Package", the models `item/package/<type>_<size>` read from the jar), `PackageItem`
(`getDescriptionId` is `item.create.package` / `item.create.rare_package` whatever the style),
`AllItems.PACKAGE_FILTER` (`FilterItem.address`), `recipe/crafting/kinetics/package_filter`.
Packages have no recipe: a Packager makes them. The behaviour is
`native/api/src/machines/package.rs` (known differences at its top).
"""

# `PackageStyles.STYLES`, in Create's order.
STYLES = [
    ("cardboard_package_12x12", False), ("cardboard_package_10x12", False),
    ("cardboard_package_10x8", False), ("cardboard_package_12x10", False),
] + [(f"rare_{name}_package", True) for name in (
    "creeper", "darcy", "evan", "jinx", "kryppers", "simi", "starlotte", "thunder", "up", "vector")]

ITEMS = {name: {"template": "minecraft:iron_ingot", "max_stack_size": 1} for name, _ in STYLES}
ITEMS["package_filter"] = {"template": "minecraft:iron_ingot", "max_stack_size": 1}

# Each item has its own key here, but they all read as the two `PackageItem.getDescriptionId`s.
NAMES = {f"item.create.{name}": "Rare Package" if rare else "Cardboard Package" for name, rare in STYLES}
NAMES["item.create.package_filter"] = "Package Filter"

TAB = [name for name, _ in STYLES] + ["package_filter"]

TAGS = {
    # `AllItemTags.PACKAGES`.
    "create:item/packages": [f"create:{name}" for name, _ in STYLES],
}

RECIPES = {
    # `data/create/recipe/crafting/kinetics/package_filter.json`.
    "crafting/kinetics/package_filter": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["ASA"],
        "key": {"A": "#c:nuggets/zinc", "S": "#minecraft:wool"},
        "result": {"id": "create:package_filter"},
    },
    # `recipe/crafting/appliances/package_filter_clear`: a filter alone gives a clean one.
    "crafting/appliances/package_filter_clear": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:package_filter"],
        "result": {"id": "create:package_filter"},
    },
}
