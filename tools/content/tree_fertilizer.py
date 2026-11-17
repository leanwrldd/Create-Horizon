"""Tree Fertilizer: `create:tree_fertilizer`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.TREE_FERTILIZER` (`TreeFertilizerItem`, a plain item that
stacks to 64). Create's only recipe for it is `recipe/crafting/tree_fertilizer`, which is guarded
by `neoforge:mod_loaded(upgrade_aquatic)` and needs that mod's corals: left out. The item model
is Create's `item/tree_fertilizer`. The behaviour is `native/api/src/machines/tree_fertilizer.rs`.
"""

ITEMS = {"tree_fertilizer": {"template": "minecraft:iron_ingot", "max_stack_size": 64}}

NAMES = {"item.create.tree_fertilizer": "Tree Fertilizer"}

TAB = ["tree_fertilizer"]
