"""Backtanks and diving gear (modules `backtank` and `diving`): `create:copper_backtank`,
`create:netherite_backtank` (a worn chestplate and a placed block), `create:copper_diving_helmet`,
`create:netherite_diving_helmet`, `create:copper_diving_boots`, `create:netherite_diving_boots`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.COPPER_BACKTANK` and `NETHERITE_BACKTANK`
(`SharedProperties.copperMetal` / `netheriteMetal`, `BuilderTransformers.backtank`: `pickaxeOnly`,
`CStress.setImpact(4.0)`, drops its item with the air; `BacktankBlock`: `facing`; `AllShapes.BACKTANK`
`shape(3, 0, 3, 13, 12, 13).add(SIX_VOXEL_POLE)`), `AllItems.COPPER_BACKTANK`/`NETHERITE_BACKTANK` (a
`BacktankItem` chestplate that places its block when used on one: `place_block` here),
`COPPER_DIVING_HELMET`/`BOOTS` (`durability(Type.HELMET.getDurability(7))`: 77 and 91) and the netherite
ones (37: 407 and 481, as vanilla's netherite), `AllArmorMaterials.COPPER` (`{2, 4, 3, 1}` for helmet,
chestplate, leggings, boots, enchantability 7, repaired with copper ingots), the tag
`create:pressurized_air_sources`, and the recipes under `crafting/appliances/`. The worn items are made
from iron and netherite armour (see the module headers for what that changes).
"""
from gen_content import HORIZONTAL_FACING_STATE, block

ORANGE = 15  # `Blocks.COPPER_BLOCK`'s map colour
BLACK = 29  # `Blocks.NETHERITE_BLOCK`'s
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}  # SharedProperties.copperMetal
NETHERITE = {"hardness": 50.0, "explosion_resistance": 1200.0, "requires_tool": True,
             "sounds": "minecraft:netherite_block"}  # SharedProperties.netheriteMetal

_S = 1 / 16
# `AllShapes.BACKTANK`: the body and the pole out of the top.
_SHAPES = {"default": [[3 * _S, 0, 3 * _S, 13 * _S, 12 * _S, 13 * _S], [5 * _S, 0, 5 * _S, 11 * _S, 1, 11 * _S]]}


def _tank(base, colour):
    # The body is Create's `block/<name>/block` in the chunk mesh; the entity draws the shaft and cogs.
    tank = block(HORIZONTAL_FACING_STATE, {"facing": "north"}, base, colour, _SHAPES, render_shape="MODEL")
    tank["item"] = False  # the worn item places it
    tank["properties"]["occludes"] = False
    return tank


BLOCKS = {"copper_backtank": _tank(COPPER, ORANGE), "netherite_backtank": _tank(NETHERITE, BLACK)}
PICKAXE_ONLY = {"copper_backtank", "netherite_backtank"}


def _armor(slot, amount):
    return {"type": "minecraft:armor", "id": f"minecraft:armor.{slot}", "amount": float(amount),
            "operation": "add_value", "slot": {"head": "head", "chestplate": "chest", "boots": "feet"}[slot]}


_COPPER_ARMOUR = {"minecraft:enchantable": {"value": 7}}

ITEMS = {
    # `BacktankItem`: not damageable; chest armour 4 (copper) or netherite's own.
    "copper_backtank": {
        "template": "minecraft:iron_chestplate", "max_stack_size": 1,
        "place_block": "create:copper_backtank",
        "components": {**_COPPER_ARMOUR, "!minecraft:max_damage": {}, "!minecraft:repairable": {},
                       "minecraft:attribute_modifiers": [_armor("chestplate", 4)]},
    },
    "netherite_backtank": {
        "template": "minecraft:netherite_chestplate", "max_stack_size": 1,
        "place_block": "create:netherite_backtank",
        "components": {"!minecraft:max_damage": {}, "!minecraft:repairable": {}},
    },
    "copper_diving_helmet": {
        "template": "minecraft:iron_helmet",
        "components": {**_COPPER_ARMOUR, "minecraft:max_damage": 77,
                       "minecraft:repairable": {"items": "minecraft:copper_ingot"},
                       "minecraft:attribute_modifiers": [_armor("head", 2)]},
    },
    "netherite_diving_helmet": {"template": "minecraft:netherite_helmet"},
    "copper_diving_boots": {
        "template": "minecraft:iron_boots",
        "components": {**_COPPER_ARMOUR, "minecraft:max_damage": 91,
                       "minecraft:repairable": {"items": "minecraft:copper_ingot"},
                       "minecraft:attribute_modifiers": [_armor("boots", 1)]},
    },
    "netherite_diving_boots": {"template": "minecraft:netherite_boots"},
}

NAMES = {
    "item.create.copper_backtank": "Copper Backtank",
    "item.create.netherite_backtank": "Netherite Backtank",
    "item.create.copper_diving_helmet": "Copper Diving Helmet",
    "item.create.netherite_diving_helmet": "Netherite Diving Helmet",
    "item.create.copper_diving_boots": "Copper Diving Boots",
    "item.create.netherite_diving_boots": "Netherite Diving Boots",
    "block.create.copper_backtank": "Copper Backtank",
    "block.create.netherite_backtank": "Netherite Backtank",
    "tag.item.create.pressurized_air_sources": "Pressurized Air Sources",
}

# The partial models the block entity draws (`AllPartialModels.COPPER_BACKTANK_COGS`, `..._SHAFT`).
MODELS = [f"block/{name}/{part}" for name in ("copper_backtank", "netherite_backtank")
          for part in ("block_cogs", "block_shaft_input")]

RECIPES = {
    # `crafting/appliances/copper_backtank`, `copper_diving_helmet`, `copper_diving_boots`.
    "crafting/appliances/copper_backtank": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["AGA", "PBP", " P "],
        "key": {"A": "create:andesite_alloy", "B": "#c:storage_blocks/copper", "G": "create:shaft",
                "P": "#c:ingots/copper"},
        "result": {"id": "create:copper_backtank", "count": 1},
    },
    "crafting/appliances/copper_diving_helmet": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["PPP", "PGP"],
        "key": {"G": "#c:glass_blocks", "P": "#c:ingots/copper"},
        "result": {"id": "create:copper_diving_helmet", "count": 1},
    },
    "crafting/appliances/copper_diving_boots": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["P P", "P P", "G G"],
        "key": {"G": "create:andesite_alloy", "P": "#c:ingots/copper"},
        "result": {"id": "create:copper_diving_boots", "count": 1},
    },
    # The netherite ones are smithing upgrades (`..._from_netherite` for a backtank on a netherite chestplate).
    "crafting/appliances/netherite_backtank": {
        "type": "minecraft:smithing_transform",
        "template": "minecraft:netherite_upgrade_smithing_template", "base": "create:copper_backtank",
        "addition": "#c:ingots/netherite", "result": {"id": "create:netherite_backtank"},
    },
    "crafting/appliances/netherite_backtank_from_netherite": {
        "type": "minecraft:smithing_transform",
        "template": "minecraft:netherite_upgrade_smithing_template", "base": "minecraft:netherite_chestplate",
        "addition": "create:copper_backtank", "result": {"id": "create:netherite_backtank"},
    },
    "crafting/appliances/netherite_diving_helmet": {
        "type": "minecraft:smithing_transform",
        "template": "minecraft:netherite_upgrade_smithing_template", "base": "create:copper_diving_helmet",
        "addition": "#c:ingots/netherite", "result": {"id": "create:netherite_diving_helmet"},
    },
    "crafting/appliances/netherite_diving_boots": {
        "type": "minecraft:smithing_transform",
        "template": "minecraft:netherite_upgrade_smithing_template", "base": "create:copper_diving_boots",
        "addition": "#c:ingots/netherite", "result": {"id": "create:netherite_diving_boots"},
    },
}

TAGS = {
    # `AllItemTags.PRESSURIZED_AIR_SOURCES`.
    "create:item/pressurized_air_sources": ["create:copper_backtank", "create:netherite_backtank"],
    "minecraft:item/chest_armor": ["create:copper_backtank", "create:netherite_backtank"],
    "minecraft:item/head_armor": ["create:copper_diving_helmet", "create:netherite_diving_helmet"],
    "minecraft:item/foot_armor": ["create:copper_diving_boots", "create:netherite_diving_boots"],
    # NeoForge's `c:storage_blocks/copper` and `c:ingots/netherite`.
    "c:item/storage_blocks/copper": ["minecraft:copper_block"],
    "c:item/ingots/netherite": ["minecraft:netherite_ingot"],
}

TAB = ["copper_backtank", "netherite_backtank", "copper_diving_helmet", "netherite_diving_helmet",
       "copper_diving_boots", "netherite_diving_boots"]
