"""The Fluid Tank and the Creative Fluid Tank: `create:fluid_tank`, `create:creative_fluid_tank`.

Create: `AllBlocks.FLUID_TANK` and `CREATIVE_FLUID_TANK` (`SharedProperties.copperMetal`: copper
block's 3.0 and 6.0, `SoundType.COPPER`; `noOcclusion`; `pickaxeOnly`), `FluidTankBlock`
(`TOP`, `BOTTOM` and `SHAPE`), `FluidTankGenerator`, `recipe/crafting/kinetics/fluid_tank`.
The blocks are in the chunk mesh (Create's own `top` / `bottom` / `shape` variants); the
controller's block entity draws the fluid.
"""

from gen_content import PURPLE, block

# `Blocks.COPPER_BLOCK`: `MapColor.COLOR_ORANGE`.
ORANGE = 15
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}

# `FluidTankBlock.Shape`, in its order.
STATES = {
    "top": {"type": "boolean"},
    "bottom": {"type": "boolean"},
    "shape": {"type": "enum",
              "values": ["plain", "window", "window_nw", "window_sw", "window_ne", "window_se"]},
}
# The constructor's `registerDefaultState`: a lone tank, with a window.
DEFAULT = {"top": "true", "bottom": "true", "shape": "window"}


def _tank(map_color, components=None):
    value = block(STATES, DEFAULT, COPPER, map_color, render_shape="MODEL")
    value["properties"]["occludes"] = False  # `noOcclusion()`
    if components:
        value["components"] = components
    return value


BLOCKS = {
    "fluid_tank": _tank(ORANGE),
    # `.properties(p -> p.rarity(Rarity.EPIC))` on its item, `MapColor.COLOR_PURPLE`.
    "creative_fluid_tank": _tank(PURPLE, {"minecraft:rarity": "epic"}),
}
PICKAXE_ONLY = {"fluid_tank", "creative_fluid_tank"}
NAMES = {
    "block.create.fluid_tank": "Fluid Tank",
    "block.create.creative_fluid_tank": "Creative Fluid Tank",
}
TAB = ["fluid_tank", "creative_fluid_tank"]

RECIPES = {
    # `recipe/crafting/kinetics/fluid_tank`: the Creative Fluid Tank has none.
    "fluid_tank": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "B"],
        "key": {"B": "#c:plates/copper", "C": "#c:barrels/wooden"},
        "result": {"id": "create:fluid_tank"},
    },
}
# What NeoForge's `c:barrels/wooden` holds of vanilla's (`Tags.Items.BARRELS_WOODEN`).
TAGS = {"c:item/barrels/wooden": ["minecraft:barrel"]}
