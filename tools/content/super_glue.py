"""Super Glue (module `super_glue`).

Create `mc1.21.1/dev` at 4148fd8: `AllItems.SUPER_GLUE` (`stacksTo(1)`, `durability(99)`) and
`recipe/crafting/kinetics/super_glue`. The behaviour is `native/api/src/machines/super_glue.rs`.
Create's entity type (`AllEntityTypes.SUPER_GLUE`) has no counterpart here: a glue is saved in a
level attachment.
"""

ITEMS = {
    "super_glue": {
        "template": "minecraft:iron_ingot",
        "max_stack_size": 1,
        # `Item.Properties.durability(99)`.
        "components": {"minecraft:max_damage": 99},
    },
}

TAB = ["super_glue"]

NAMES = {"item.create.super_glue": "Super Glue"}

RECIPES = {
    # `crafting/kinetics/super_glue`, in Horizon's 26.3 form.
    "crafting/kinetics/super_glue": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["AS", "NA"],
        "key": {"A": "#c:slimeballs", "N": "#c:nuggets/iron", "S": "#c:plates/iron"},
        "result": {"id": "create:super_glue", "count": 1},
    },
}

# NeoForge's `c:slimeballs` (`Tags.Items.SLIMEBALLS`) of vanilla's items.
TAGS = {"c:item/slimeballs": ["minecraft:slime_ball"]}
