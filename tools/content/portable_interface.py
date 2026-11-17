"""The Portable Storage Interface and the Portable Fluid Interface:
`create:portable_storage_interface` and `create:portable_fluid_interface`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.PORTABLE_STORAGE_INTERFACE` (`SharedProperties.stone`,
`MapColor.PODZOL`, `axeOrPickaxe`) and `PORTABLE_FLUID_INTERFACE` (`SharedProperties.copperMetal`,
`MapColor.TERRACOTTA_LIGHT_GRAY`, `axeOrPickaxe`), both `WrenchableDirectionalBlock`s
(`facing`), `AllShapes.PORTABLE_STORAGE_INTERFACE = shape(0, 0, 0, 16, 14, 16).forDirectional()`,
their items tagged `create:contraption_controlled`, `AllPartialModels.PORTABLE_*_INTERFACE_MIDDLE`,
`_MIDDLE_POWERED` and `_TOP`, and `recipe/crafting/kinetics/portable_storage_interface` and
`portable_fluid_interface`. The bases are in the chunk mesh (Create's own `block` model per facing);
the block entity draws the middle and the top. The behaviour is
`native/api/src/machines/portable_interface*.rs`.
"""

from gen_content import ANDESITE, FACING_STATE, PODZOL, block, facing_shapes

# `Blocks.COPPER_BLOCK`'s hardness, resistance and sound (`SharedProperties.copperMetal`).
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}
LIGHT_GRAY_TERRACOTTA = 44  # `MapColor.TERRACOTTA_LIGHT_GRAY`

# `AllShapes.PORTABLE_STORAGE_INTERFACE` facing up: the base without the part that comes out.
_SHAPES = facing_shapes([[0, 0, 0, 1, 14 / 16, 1]])

BLOCKS = {
    "portable_storage_interface": block(FACING_STATE, {"facing": "north"}, ANDESITE, PODZOL,
                                        _SHAPES, render_shape="MODEL"),
    "portable_fluid_interface": block(FACING_STATE, {"facing": "north"}, COPPER,
                                      LIGHT_GRAY_TERRACOTTA, _SHAPES, render_shape="MODEL"),
}
NAMES = {
    "block.create.portable_storage_interface": "Portable Storage Interface",
    "block.create.portable_fluid_interface": "Portable Fluid Interface",
}
TAB = ["portable_storage_interface", "portable_fluid_interface"]
TAGS = {"create:item/contraption_controlled": ["create:portable_storage_interface",
                                               "create:portable_fluid_interface"]}

MODELS = [f"block/{family}/{part}"
          for family in ("portable_storage_interface", "portable_fluid_interface")
          for part in ("block_middle", "block_middle_powered", "block_top")]

RECIPES = {
    # `crafting/kinetics/portable_storage_interface` and `portable_fluid_interface`.
    "crafting/kinetics/portable_storage_interface": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:andesite_casing", "create:chute"],
        "result": {"id": "create:portable_storage_interface", "count": 1},
    },
    "crafting/kinetics/portable_fluid_interface": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:copper_casing", "create:chute"],
        "result": {"id": "create:portable_fluid_interface", "count": 1},
    },
}
