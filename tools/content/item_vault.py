"""The Item Vault: `create:item_vault`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ITEM_VAULT` (`SharedProperties.softMetal`: gold block's
3.0 and 6.0, `requiresCorrectToolForDrops`; `MapColor.TERRACOTTA_BLUE`, `SoundType.NETHERITE_BLOCK`,
`explosionResistance(1200)`, `pickaxeOnly`), `ItemVaultBlock` (`HORIZONTAL_AXIS`, named `axis`, and
`LARGE`), the blockstate Create's datagen writes (the one model turned about y by the axis), and
`data/create/recipe/crafting/kinetics/item_vault.json`. The model is the vault's single-block one,
imported from Create's jar; its connected textures (`ItemVaultCTBehaviour`) are not drawn. The
behaviour is `native/api/src/machines/item_vault.rs` (known differences at its top).
"""

# `MapColor.TERRACOTTA_BLUE`.
TERRACOTTA_BLUE = 47

BLOCKS = {
    "item_vault": {
        "template": "minecraft:gold_block",
        "item": True,
        "properties": {
            "hardness": 3.0, "explosion_resistance": 1200.0, "requires_tool": True,
            "map_color": TERRACOTTA_BLUE, "sounds": "minecraft:netherite_block",
        },
        # `ItemVaultBlock`: `HORIZONTAL_AXIS` (`axis`: x, z) and `LARGE`; the constructor's
        # `registerDefaultState` gives `large=false`, the axis its first value.
        "state_properties": {
            "axis": {"type": "enum", "values": ["x", "z"]},
            "large": {"type": "boolean"},
        },
        "default_state": {"axis": "x", "large": "false"},
    },
}

PICKAXE_ONLY = {"item_vault"}
NAMES = {"block.create.item_vault": "Item Vault"}
TAB = ["item_vault"]

RECIPES = {
    # `recipe/crafting/kinetics/item_vault`: an iron sheet, a wooden barrel and an iron sheet.
    "crafting/kinetics/item_vault": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "B"],
        "key": {"B": "#c:plates/iron", "C": "#c:barrels/wooden"},
        "result": {"id": "create:item_vault"},
    },
}
# What NeoForge's `c:barrels/wooden` holds of vanilla's (`Tags.Items.BARRELS_WOODEN`).
TAGS = {"c:item/barrels/wooden": ["minecraft:barrel"]}
