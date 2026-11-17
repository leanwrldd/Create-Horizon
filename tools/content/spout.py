"""The Spout: `create:spout` and every `create:filling` recipe it fills by.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.SPOUT` (`SharedProperties.copperMetal`, pickaxe only,
`partialBaseModel`, `AssetLookup`), `SpoutBlock` (`AllShapes.SPOUT`), `AllPartialModels.SPOUT_TOP`,
`SPOUT_MIDDLE`, `SPOUT_BOTTOM`, `recipe/crafting/kinetics/spout`, `AllSoundEvents.SPOUTING`'s subtitle,
and the datagen `recipe/filling/*` with what depends on another mod left out. The three that take a
`create:potion` fluid (`filling/glowstone`, `gunpowder`, `redstone`: 25 mB of a Night Vision, Harming or Strength
potion, written as a `neoforge:components` ingredient) are in: the fluid is `rules/potion_fluid.rs`'s.
The behaviour is `native/api/src/machines/spout.rs`.
"""

from gen_content import block

ORANGE = 15  # `Blocks.COPPER_BLOCK`'s `MapColor.COLOR_ORANGE`
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}  # SharedProperties.copperMetal

BLOCKS = {
    # `AllShapes.SPOUT`: shape(1, 2, 1, 15, 14, 15).add(2, 0, 2, 14, 16, 14). The base casing is in
    # the chunk mesh; the block entity draws the nozzle and the fluid.
    "spout": block({}, {}, COPPER, ORANGE, {"default": [
        [1 / 16, 2 / 16, 1 / 16, 15 / 16, 14 / 16, 15 / 16],
        [2 / 16, 0, 2 / 16, 14 / 16, 1, 14 / 16]]}, render_shape="MODEL"),
}
PICKAXE_ONLY = {"spout"}
TAB = ["spout"]
# `AllPartialModels.SPOUT_TOP`, `SPOUT_MIDDLE`, `SPOUT_BOTTOM`.
MODELS = ["block/spout/top", "block/spout/middle", "block/spout/bottom"]
NAMES = {
    "block.create.spout": "Spout",
    "create.subtitle.spout": "Spout spurts",
}


def _filling(item, fluid, result, tag=False):
    """The datagen's `FillingRecipe` JSON: one item and one fluid in, one item out."""
    ingredient = ({"type": "neoforge:tag", "amount": fluid[1], "tag": fluid[0]} if tag else
                  {"type": "neoforge:single", "amount": fluid[1], "fluid": fluid[0]})
    return {"type": "create:filling", "ingredients": [{"item": item}, ingredient],
            "results": [{"id": result}]}


def _potion_filling(potion, result):
    """`recipe/filling/glowstone` and its two siblings: cinder flour and 25 mB of a regular potion."""
    return {"type": "create:filling", "ingredients": [
        {"item": "create:cinder_flour"},
        {"type": "neoforge:components", "amount": 25, "fluids": "create:potion",
         "components": {"create:potion_fluid_bottle_type": "regular",
                        "minecraft:potion_contents": {"potion": potion}}}],
            "results": [{"id": result}]}


RECIPES = {
    # `recipe/crafting/kinetics/spout`.
    "crafting/kinetics/spout": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["T", "P"],
        "key": {"T": "create:copper_casing", "P": "minecraft:dried_kelp"},
        "result": {"id": "create:spout", "count": 1},
    },
    "filling/blaze_cake": _filling("create:blaze_cake_base", ("minecraft:lava", 250),
                                   "create:blaze_cake"),
    "filling/builders_tea": _filling("minecraft:glass_bottle", ("create:tea", 250),
                                     "create:builders_tea"),
    "filling/chocolate_glazed_berries": _filling("minecraft:sweet_berries",
                                                 ("create:chocolate", 250),
                                                 "create:chocolate_glazed_berries"),
    "filling/glowstone": _potion_filling("minecraft:night_vision", "minecraft:glowstone_dust"),
    "filling/grass_block": _filling("minecraft:dirt", ("minecraft:water", 500),
                                    "minecraft:grass_block"),
    "filling/gunpowder": _potion_filling("minecraft:harming", "minecraft:gunpowder"),
    "filling/honey_bottle": _filling("minecraft:glass_bottle", ("c:honey", 250),
                                     "minecraft:honey_bottle", tag=True),
    "filling/honeyed_apple": _filling("minecraft:apple", ("c:honey", 250),
                                      "create:honeyed_apple", tag=True),
    "filling/redstone": _potion_filling("minecraft:strength", "minecraft:redstone"),
    "filling/sweet_roll": _filling("minecraft:bread", ("c:milk", 250), "create:sweet_roll",
                                   tag=True),
}
