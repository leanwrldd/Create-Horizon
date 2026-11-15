"""The Blaze Burner: `create:blaze_burner`, `create:lit_blaze_burner` and `create:empty_blaze_burner`.

Create: `AllBlocks.BLAZE_BURNER` (`SharedProperties.softMetal`, `MapColor.COLOR_GRAY`,
`lightLevel(BlazeBurnerBlock::getLight)`, `pickaxeOnly`, `BlazeBurnerBlock.buildLootTable`),
`LIT_BLAZE_BURNER` (`LitBlazeBurnerBlock.getLight`, drops the empty burner),
`AllItems.EMPTY_BLAZE_BURNER`, `AllShapes.HEATER_BLOCK_SHAPE`,
`recipe/crafting/kinetics/empty_blaze_burner`, `data/create/tags/block/passive_boiler_heaters`.
"""

from gen_content import GRAY, block

HEAT_LEVELS = ["none", "smouldering", "fading", "kindled", "seething"]  # `HeatLevel`
# `SharedProperties.softMetal`: a gold block.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:gold_block"}
LIGHT_GRAY = 22  # `MapColor.COLOR_LIGHT_GRAY`
# `AllShapes.HEATER_BLOCK_SHAPE = shape(1, 0, 1, 15, 14, 15)`.
SHAPE = {"default": [[1 / 16, 0, 1 / 16, 15 / 16, 14 / 16, 15 / 16]]}

_burner = block(
    {"blaze": {"type": "enum", "values": HEAT_LEVELS},
     "facing": {"type": "enum", "values": ["north", "south", "west", "east"]}},
    {"blaze": "none", "facing": "north"}, SOFT_METAL, GRAY, SHAPE, render_shape="MODEL")
# `BlazeBurnerBlock.getLight`: 0, 8, then 15.
_burner["state_light"] = {"property": "blaze", "values": {
    "none": 0, "smouldering": 8, "fading": 15, "kindled": 15, "seething": 15}}

_lit = block(
    {"flame_type": {"type": "enum", "values": ["regular", "soul"]}},
    {"flame_type": "regular"}, SOFT_METAL, LIGHT_GRAY, SHAPE, render_shape="MODEL")
# `LitBlazeBurnerBlock.getLight`: 12, or 9 for soul fire.
_lit["state_light"] = {"property": "flame_type", "values": {"regular": 12, "soul": 9}}
# The lit burner has no item of its own (`AllBlocks.LIT_BLAZE_BURNER` has no `.item()`).
_lit["item"] = False

BLOCKS = {"blaze_burner": _burner, "lit_blaze_burner": _lit}
# `EMPTY_BLAZE_BURNER` is a `BlazeBurnerBlockItem` for the burner block with no blaze in it.
ITEMS = {"empty_blaze_burner": {"template": "minecraft:iron_ingot", "max_stack_size": 64,
                                "place_block": "create:blaze_burner"}}
PICKAXE_ONLY = {"blaze_burner", "lit_blaze_burner"}
NAMES = {
    "block.create.blaze_burner": "Blaze Burner",
    "block.create.lit_blaze_burner": "Lit Blaze Burner",
    "item.create.empty_blaze_burner": "Empty Blaze Burner",
}
TAB = ["empty_blaze_burner", "blaze_burner"]
# `AllPartialModels.BLAZE_*` and `BLAZE_BURNER_*` (`BlazeBurnerRenderer`).
MODELS = [f"block/blaze_burner/{name}" for name in (
    "blaze/inert", "blaze/idle", "blaze/active", "blaze/super", "blaze/super_active",
    "rods_small", "rods_large", "superheated_rods_small", "superheated_rods_large")]

# `BlazeBurnerBlock.buildLootTable`: the empty burner while no blaze is in it, else the burner.
_SURVIVES = {"type": "minecraft:survives_explosion"}
_NO_BLAZE = {"type": "minecraft:block_state_property", "block": "create:blaze_burner",
             "properties": {"blaze": "none"}}
LOOT = {
    "blaze_burner": [
        {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "create:empty_blaze_burner"}],
         "condition": {"type": "minecraft:all_of", "terms": [_SURVIVES, _NO_BLAZE]}},
        {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "create:blaze_burner"}],
         "condition": {"type": "minecraft:all_of", "terms": [
             _SURVIVES, {"type": "minecraft:inverted", "term": _NO_BLAZE}]}},
    ],
    # `lt.dropOther(block, EMPTY_BLAZE_BURNER)`.
    "lit_blaze_burner": [
        {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "create:empty_blaze_burner"}],
         "condition": _SURVIVES},
    ],
}

RECIPES = {
    # `recipe/crafting/kinetics/empty_blaze_burner`.
    "empty_blaze_burner": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" I ", "IAI", " I "],
        "key": {"I": "#c:plates/iron", "A": "#c:netherracks"},
        "result": {"id": "create:empty_blaze_burner"},
    },
}
TAGS = {
    # What NeoForge's `c:netherracks` holds.
    "c:item/netherracks": ["minecraft:netherrack"],
    # `data/create/tags/block/passive_boiler_heaters.json`.
    "create:block/passive_boiler_heaters": [
        "create:blaze_burner", "create:lit_blaze_burner", "minecraft:magma_block", "minecraft:lava",
        "#minecraft:campfires", "#minecraft:fire"],
}
