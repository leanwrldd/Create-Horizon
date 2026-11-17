"""Minecart Coupling: `create:minecart_coupling`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.MINECART_COUPLING` (`MinecartCouplingItem`, stacks to 64),
`recipe/crafting/curiosities/minecart_coupling`. The item model is Create's `item/minecart_coupling`.
The behaviour is `native/api/src/machines/coupling.rs`.
"""

ITEMS = {"minecart_coupling": {"template": "minecraft:iron_ingot", "max_stack_size": 64}}

NAMES = {
    "item.create.minecart_coupling": "Minecart Coupling",
    "create.minecart_coupling.no_loops": "Couplings cannot form a loop",
    "create.minecart_coupling.removed": "Removed all couplings from minecart",
    "create.minecart_coupling.too_far": "Minecarts are too far apart",
    "create.minecart_coupling.two_couplings_max": "Minecarts cannot have more than two couplings each",
    "create.minecart_coupling.unloaded": "Parts of your train seem to be in unloaded chunks",
}

TAB = ["minecart_coupling"]

RECIPES = {
    "crafting/curiosities/minecart_coupling": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["E", "O", "E"],
        "key": {"E": "create:andesite_alloy", "O": "#c:plates/iron"},
        "result": {"id": "create:minecart_coupling"},
    },
}
