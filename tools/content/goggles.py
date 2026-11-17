"""Engineer's Goggles (module `goggles`; the overlay is `native/api/src/machines/goggles.rs`).

Create `mc1.21.1/dev` at 4148fd8: `AllItems.GOGGLES` (`GogglesItem`, `stacksTo(1)`, lang "Engineer's
Goggles"), `GogglesItem` (`Equipable`, `EquipmentSlot.HEAD`; right-click swaps it with the helmet),
`GogglesModel` (`ItemDisplayContext.HEAD` uses the partial model `block/goggles`, which has its own
`head` transform), `recipe/crafting/kinetics/goggles`.

Worn in the head slot as an `equippable` component with no armour asset, so Horizon draws it as it
draws a pumpkin: the item's own model in its `head` display context (`CustomHeadLayer`). The item
definition picks `create:block/goggles` there and the flat item everywhere else, as `GogglesModel`
does.
"""

ITEMS = {
    "goggles": {
        "template": "minecraft:iron_ingot",
        "max_stack_size": 1,
        # `GogglesItem.getEquipmentSlot`: the head. Create's equip sound is the generic one.
        "components": {"minecraft:equippable": {"slot": "head"}},
    },
}

NAMES = {"item.create.goggles": "Engineer's Goggles"}

ITEM_MODELS = {
    "goggles": {
        "model": {
            "type": "minecraft:select",
            "property": "minecraft:display_context",
            "cases": [{
                "when": "head",
                "model": {"type": "minecraft:model", "model": "create:block/goggles"},
            }],
            "fallback": {"type": "minecraft:model", "model": "create:item/goggles"},
        },
    },
}

RECIPES = {
    # `data/create/recipe/crafting/kinetics/goggles.json`.
    "crafting/kinetics/goggles": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" S ", "GPG"],
        "key": {"G": "#c:glass_blocks", "P": "#c:plates/gold", "S": "#c:strings"},
        "result": {"id": "create:goggles", "count": 1},
    },
}

TAB = ["goggles"]
