"""The Item Drain: `create:item_drain` and the `create:emptying` recipes it empties by.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ITEM_DRAIN` (`SharedProperties.copperMetal`, pickaxe
only, `simpleItem`), `ItemDrainBlock` (`AllShapes.CASING_13PX` facing up),
`recipe/crafting/kinetics/item_drain`, and the datagen `recipe/emptying/*` (the other mods' are left
out).
"""

from gen_content import block

ORANGE = 15  # `Blocks.COPPER_BLOCK`'s `MapColor.COLOR_ORANGE`
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}  # SharedProperties.copperMetal

BLOCKS = {
    # `AllShapes.CASING_13PX.get(UP)`. The casing is in the chunk mesh (`block/item_drain`); the
    # block entity draws the fluid, the stream and the item.
    "item_drain": block({}, {}, COPPER, ORANGE, {"default": [[0, 0, 0, 1, 13 / 16, 1]]},
                        render_shape="MODEL"),
}
PICKAXE_ONLY = {"item_drain"}
TAB = ["item_drain"]
NAMES = {"block.create.item_drain": "Item Drain"}


def _emptying(item, empty, fluid):
    """The datagen's `EmptyingRecipe` JSON: one item in, an item and a fluid out."""
    return {"type": "create:emptying", "ingredients": [{"item": item}],
            "results": [{"id": empty}, {"amount": fluid[1], "id": fluid[0]}]}


RECIPES = {
    # `recipe/crafting/kinetics/item_drain`.
    "crafting/kinetics/item_drain": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["P", "S"],
        "key": {"P": "minecraft:iron_bars", "S": "create:copper_casing"},
        "result": {"id": "create:item_drain", "count": 1},
    },
    "emptying/builders_tea": _emptying("create:builders_tea", "minecraft:glass_bottle",
                                       ("create:tea", 250)),
    "emptying/honey_bottle": _emptying("minecraft:honey_bottle", "minecraft:glass_bottle",
                                       ("create:honey", 250)),
}
