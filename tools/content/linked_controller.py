"""The Linked Controller (module `linked_controller`): `create:linked_controller`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.LINKED_CONTROLLER` (`LinkedControllerItem`, `stacksTo(1)`, the item
model `item/linked_controller` with its button partials), `recipe/crafting/appliances/linked_controller`. The
behaviour is `native/api/src/machines/linked_controller.rs`; its frequencies are saved in the item component
`create:linked_controller_items`.
"""

ITEMS = {
    "linked_controller": {"template": "minecraft:iron_ingot", "max_stack_size": 1},
}

NAMES = {"item.create.linked_controller": "Linked Controller"}

RECIPES = {
    # `crafting/appliances/linked_controller`.
    "crafting/appliances/linked_controller": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["SSS", " P ", "SSS"],
        "key": {"P": "create:redstone_link", "S": "#minecraft:wooden_buttons"},
        "result": {"id": "create:linked_controller", "count": 1},
    },
}

TAB = ["linked_controller"]
