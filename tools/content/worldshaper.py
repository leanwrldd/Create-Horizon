"""The Creative Worldshaper: `create:handheld_worldshaper`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.HANDHELD_WORLDSHAPER` (`WorldshaperItem`, `Rarity.EPIC`,
lang "Creative Worldshaper"; no recipe: it is a creative item). The item model is Create's
`item/handheld_worldshaper` (its accelerator and core partials, drawn by Create's item renderer, are
not drawn here). The behaviour is `native/api/src/machines/worldshaper.rs`.
"""

ITEMS = {"handheld_worldshaper": {"template": "minecraft:iron_ingot", "max_stack_size": 1,
                                  "rarity": "epic"}}

NAMES = {"item.create.handheld_worldshaper": "Creative Worldshaper"}

# `AllCreativeModeTabs`' base tab ends with the creative gadgets.
TAB = ["handheld_worldshaper"]
