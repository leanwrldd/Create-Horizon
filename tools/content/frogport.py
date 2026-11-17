"""The Package Frogport: `create:package_frogport`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.PACKAGE_FROGPORT` (`SharedProperties.softMetal`,
`noOcclusion`, `MapColor.TERRACOTTA_BLUE`, `SoundType.NETHERITE_BLOCK`, `pickaxeOnly`, a `PackagePortItem`),
`FrogportBlock` (`AllShapes.PACKAGE_PORT`: `shape(0, 0, 0, 16, 4, 16).add(2, 2, 2, 14, 14, 14)`, no
state), the blockstate (the one model `block/package_frogport/block`), `AllPartialModels.FROGPORT_BODY`,
`_HEAD`, `_HEAD_GOGGLES`, `_TONGUE`, `recipe/crafting/logistics/package_frogport`. The behaviour is
`native/api/src/machines/frogport.rs` (known differences at its top).
"""
from gen_content import block

TERRACOTTA_BLUE = 47  # MapColor.TERRACOTTA_BLUE
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True}

BLOCKS = {
    "package_frogport": block(
        None, None, SOFT_METAL, TERRACOTTA_BLUE,
        {"default": [[0, 0, 0, 1, 4 / 16, 1], [2 / 16, 2 / 16, 2 / 16, 14 / 16, 14 / 16, 14 / 16]]},
        sounds="minecraft:netherite_block", render_shape="MODEL"),
}
PICKAXE_ONLY = {"package_frogport"}

MODELS = ["block/package_frogport/body", "block/package_frogport/head",
          "block/package_frogport/head_goggles", "block/package_frogport/tongue"]

NAMES = {"block.create.package_frogport": "Package Frogport"}
TAB = ["package_frogport"]

RECIPES = {
    "crafting/logistics/package_frogport": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "A", "C"],
        "key": {"A": "create:item_vault", "B": "#c:slimeballs", "C": "create:andesite_alloy"},
        "result": {"id": "create:package_frogport", "count": 1},
    },
}
