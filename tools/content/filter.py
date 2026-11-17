"""The List Filter and the Attribute Filter: `create:filter`, `create:attribute_filter`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.FILTER` (`FilterItem.regular`, `stacksTo(1)`) and
`ATTRIBUTE_FILTER` (`FilterItem.attribute`), `recipe/crafting/kinetics/filter`, `attribute_filter`,
`recipe/crafting/appliances/filter_clear` and `attribute_filter_clear`. The item models are
Create's `item/filter` and `item/attribute_filter`, read from the jar. The behaviour is
`native/api/src/machines/filter.rs`; the Package Filter is `package.py`.
"""

ITEMS = {name: {"template": "minecraft:iron_ingot", "max_stack_size": 1}
         for name in ("filter", "attribute_filter")}

NAMES = {
    "item.create.filter": "List Filter",
    "item.create.attribute_filter": "Attribute Filter",
}

# `AllItems`: just after the Wrench's neighbours (the kinetics tab lists them in this order).
TAB = ["filter", "attribute_filter"]

RECIPES = {
    "crafting/kinetics/filter": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["ASA"],
        "key": {"A": "#c:nuggets/iron", "S": "#minecraft:wool"},
        "result": {"id": "create:filter"},
    },
    "crafting/kinetics/attribute_filter": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["ASA"],
        "key": {"A": "#c:nuggets/brass", "S": "#minecraft:wool"},
        "result": {"id": "create:attribute_filter"},
    },
    # Crafting a filter alone gives a clean one.
    "crafting/appliances/filter_clear": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:filter"],
        "result": {"id": "create:filter"},
    },
    "crafting/appliances/attribute_filter_clear": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:attribute_filter"],
        "result": {"id": "create:attribute_filter"},
    },
}
