"""Materials: the plain items and blocks of Create's `AllItems`/`AllBlocks` that need no code.

Declarations only: ingredients, metals, nuggets, sheets, foods, cardboard, the ores and storage
blocks, the casings, their recipes of vanilla types, tags, loot and the creative tab entries, plus
the zinc ore's world generation. Every value is named after the Create class it comes from.

Known differences from Create (for the README):
- Connected textures on the casings are not declared: Horizon reads OptiFine CTM rules that need
  one image per tile, and Create's connected sprites are single sheets in the jar, which the
  build would have to slice.
- Experience Nugget does not turn into experience when used (`ExperienceNuggetItem`), the Experience
  Block does not shed End Rod particles (`ExperienceBlock.animateTick`), Chromatic Compound does not
  gather light or turn into Shadow Steel / Refined Radiance (`ChromaticCompoundItem`), and Shadow
  Steel and Refined Radiance neither float nor glow in the world (`NoGravMagicalDohickyItem`).
- Sequenced assembly items show no progress bar (`SequencedAssemblyItem`).
- Cardboard Blocks always place with axis x (`CardboardBlock.getStateForPlacement` follows the
  player), and are not flammable in the world: a content block cannot declare fire odds.
- Experience Block's sounds are the amethyst block's at pitch 1, Create's `DeferredSoundType` has 0.5.
- The zinc ore feature has no `create:config_filter` placement (Create's config switch).
"""

# --- shared forms ----------------------------------------------------------------------------

# A plain item copies the iron nugget, which has none of the components an ingot has (a trim
# material) and 64 as its stack size, as `Item::new` with default properties does.
PLAIN = "minecraft:iron_nugget"


def burn(ticks):
    """`ItemBuilder.burnTime(ticks)`, written as vanilla's own fuels are (`cooking/time_coal`):
    the burn time over the factor that a blast furnace or smoker doubles."""
    return {"minecraft:cooking_fuel": {
        "burn_time": {"type": "minecraft:div", "left": ticks, "right": {
            "type": "minecraft:conditional", "condition": "minecraft:block/fast_cooking",
            "on_false": "minecraft:cooking/normal_burn_time_reduction_factor",
            "on_true": "minecraft:cooking/fast_burn_time_reduction_factor"}},
        "speed_multiplier": "minecraft:cooking/speed_default"}}


def item(template=PLAIN, stack=64, rarity=None, components=None):
    value = {"template": template, "max_stack_size": stack}
    if rarity:
        value["rarity"] = rarity
    if components:
        value["components"] = components
    return value


def food(nutrition, modifier):
    """`FoodProperties.Builder().nutrition(n).saturationModifier(m)`: 26.3 stores the saturation
    itself, which 1.21.1 computed as nutrition * modifier * 2 (honey bottle: 6 * 0.1 * 2 = 1.2)."""
    return item("minecraft:apple", components={
        "minecraft:food": {"nutrition": nutrition, "saturation": round(nutrition * modifier * 2, 4)},
        "!minecraft:compostable": {},
    })


FOIL = {"minecraft:enchantment_glint_override": True}

# --- items: `AllItems.java`, in its order --------------------------------------------------------

ITEMS = {
    # `ingredient(...)` and `taggedIngredient(...)`.
    "wheat_flour": item(),
    "dough": item(),
    "cinder_flour": item(),
    "rose_quartz": item(),
    "polished_rose_quartz": item(),
    "powdered_obsidian": item(),
    "sturdy_sheet": item(),
    "propeller": item(),
    "whisk": item(),
    "brass_hand": item(),
    "crafter_slot_cover": item(),
    "electron_tube": item(),
    "transmitter": item(),
    "pulp": item(),
    # `CARDBOARD`: `burnTime(1000)`.
    "cardboard": item(components=burn(1000)),
    # `sequencedIngredient(...)`: `SequencedAssemblyItem` stacks to 1.
    "incomplete_precision_mechanism": item(stack=1),
    "unprocessed_obsidian_sheet": item(stack=1),
    "incomplete_track": item(stack=1),
    "precision_mechanism": item(),
    "blaze_cake_base": item(),
    # `BLAZE_CAKE`: `burnTime(6400)`; its blaze burner fuel data map entry belongs to the burner.
    "blaze_cake": item(components=burn(6400)),
    # `CREATIVE_BLAZE_CAKE`: `Rarity.EPIC`, `burnTime(Integer.MAX_VALUE)`.
    "creative_blaze_cake": item(rarity="epic", components=burn(2147483647)),
    # `BAR_OF_CHOCOLATE`, `SWEET_ROLL`, `CHOCOLATE_BERRIES`, `HONEYED_APPLE`.
    "bar_of_chocolate": food(6, 0.3),
    "sweet_roll": food(6, 0.8),
    "chocolate_glazed_berries": food(7, 0.8),
    "honeyed_apple": food(8, 0.8),
    # `BUILDERS_TEA` / `BuildersTeaItem`: stacksTo(16); nutrition 1, saturationModifier .6, always
    # edible; haste for 3 * 60 * 20 ticks, no particles or icon, chance 1; 42 ticks to drink, the
    # drink animation, and a glass bottle left behind.
    "builders_tea": item("minecraft:honey_bottle", stack=16, components={
        "minecraft:food": {"nutrition": 1, "saturation": 1.2, "can_always_eat": True},
        "minecraft:consumable": {
            "animation": "drink", "consume_seconds": 2.1, "has_consume_particles": False,
            "sound": "minecraft:entity.generic.drink",
            "on_consume_effects": [{"type": "minecraft:apply_effects", "effects": [
                {"id": "minecraft:haste", "amplifier": 0, "duration": 3600, "ambient": False,
                 "show_particles": False, "show_icon": False}]}]},
        "minecraft:use_remainder": {"id": "minecraft:glass_bottle"},
    }),
    "raw_zinc": item(),
    "zinc_ingot": item(),
    "brass_ingot": item(),
    # `CHROMATIC_COMPOUND`: `Rarity.UNCOMMON`; `getMaxStackSize` is 16 while it holds no light.
    "chromatic_compound": item(stack=16, rarity="uncommon"),
    # `SHADOW_STEEL`: `Rarity.UNCOMMON`.
    "shadow_steel": item(rarity="uncommon"),
    # `REFINED_RADIANCE`: `Rarity.UNCOMMON`, `isFoil`.
    "refined_radiance": item(rarity="uncommon", components=FOIL),
    "copper_nugget": item(),
    "zinc_nugget": item(),
    "brass_nugget": item(),
    # `EXP_NUGGET`: `Rarity.UNCOMMON`, `isFoil`.
    "experience_nugget": item(rarity="uncommon", components=FOIL),
    "copper_sheet": item(),
    "brass_sheet": item(),
    "golden_sheet": item(),
    "crushed_raw_iron": item(),
    "crushed_raw_gold": item(),
    "crushed_raw_copper": item(),
    "crushed_raw_zinc": item(),
}

# --- blocks: `AllBlocks.java` ------------------------------------------------------------------

# `MapColor` ids, with their `MapColor.col` in `horizon-items`' `MAP_COLORS`.
PLANT, METAL, STONE_COLOR, CLAY_BROWN = 7, 6, 11, 26
SNOW, BLACK, PODZOL, YELLOW_TERRACOTTA = 8, 29, 34, 40
LIGHT_GRAY_TERRACOTTA, BROWN_TERRACOTTA, GLOW_LICHEN = 44, 48, 61


def cube(template, hardness, resistance, map_color, sounds, requires_tool=True, light=None,
         components=None, states=None):
    """A full block on a vanilla `template`: what `initialProperties(() -> template)` copies, with
    its numbers written out, then the block's own `properties(...)`."""
    properties = {"hardness": hardness, "explosion_resistance": resistance,
                  "requires_tool": requires_tool, "map_color": map_color, "sounds": sounds}
    if light is not None:
        properties["light"] = light
    value = {"template": template, "item": True, "properties": properties}
    if components:
        value["components"] = components
    if states:
        value["state_properties"] = states
        value["default_state"] = {name: values["values"][0] for name, values in states.items()}
    return value


# `CardboardBlock`: `HORIZONTAL_AXIS` (named `axis`, values x and z).
HORIZONTAL_AXIS = {"axis": {"type": "enum", "values": ["x", "z"]}}
# `SoundType.WOOD` is named by a block (`BuilderTransformers.casing`), as in gen_content.py.
WOOD_SOUND = "minecraft:oak_planks"

BLOCKS = {
    # `BuilderTransformers.casing`: `SharedProperties::stone` (andesite: 1.5, 6.0), `SoundType.WOOD`.
    "andesite_casing": cube("minecraft:andesite", 1.5, 6.0, PODZOL, WOOD_SOUND),
    "brass_casing": cube("minecraft:andesite", 1.5, 6.0, BROWN_TERRACOTTA, WOOD_SOUND),
    # `COPPER_CASING`: `SoundType.COPPER`.
    "copper_casing": cube("minecraft:andesite", 1.5, 6.0, LIGHT_GRAY_TERRACOTTA,
                          "minecraft:copper_block"),
    "shadow_steel_casing": cube("minecraft:andesite", 1.5, 6.0, BLACK, WOOD_SOUND),
    # `REFINED_RADIANCE_CASING`: `lightLevel($ -> 12)`.
    "refined_radiance_casing": cube("minecraft:andesite", 1.5, 6.0, SNOW, WOOD_SOUND, light=12),
    # `ZINC_ORE`: `Blocks.GOLD_ORE` (3.0, 3.0), `MapColor.METAL`, `SoundType.STONE`.
    "zinc_ore": cube("minecraft:gold_ore", 3.0, 3.0, METAL, "minecraft:stone"),
    # `DEEPSLATE_ZINC_ORE`: `Blocks.DEEPSLATE_GOLD_ORE` (4.5, 3.0), `MapColor.STONE`, `SoundType.DEEPSLATE`.
    "deepslate_zinc_ore": cube("minecraft:deepslate_gold_ore", 4.5, 3.0, STONE_COLOR,
                               "minecraft:deepslate"),
    # `RAW_ZINC_BLOCK`: `Blocks.RAW_GOLD_BLOCK` (5.0, 6.0), `MapColor.GLOW_LICHEN`.
    "raw_zinc_block": cube("minecraft:raw_gold_block", 5.0, 6.0, GLOW_LICHEN, "minecraft:stone"),
    # `ZINC_BLOCK`: `Blocks.IRON_BLOCK` (5.0, 6.0), `MapColor.GLOW_LICHEN`.
    "zinc_block": cube("minecraft:iron_block", 5.0, 6.0, GLOW_LICHEN, "minecraft:iron_block"),
    # `ANDESITE_ALLOY_BLOCK`: `Blocks.ANDESITE` (1.5, 6.0), `MapColor.STONE`.
    "andesite_alloy_block": cube("minecraft:andesite", 1.5, 6.0, STONE_COLOR, "minecraft:stone"),
    # `BRASS_BLOCK`: `Blocks.IRON_BLOCK`, `MapColor.TERRACOTTA_YELLOW`.
    "brass_block": cube("minecraft:iron_block", 5.0, 6.0, YELLOW_TERRACOTTA, "minecraft:iron_block"),
    # `CARDBOARD_BLOCK` and `BOUND_CARDBOARD_BLOCK`: `Blocks.MUSHROOM_STEM` (0.2, 0.2, no tool
    # needed), `MapColor.COLOR_BROWN`, `SoundType.CHISELED_BOOKSHELF`, `ignitedByLava`; their items
    # `burnTime(4000)`. Planks stand in for the stem, which has six face states of its own.
    "cardboard_block": cube("minecraft:oak_planks", 0.2, 0.2, CLAY_BROWN,
                            "minecraft:chiseled_bookshelf", requires_tool=False,
                            components=burn(4000), states=HORIZONTAL_AXIS),
    "bound_cardboard_block": cube("minecraft:oak_planks", 0.2, 0.2, CLAY_BROWN,
                                  "minecraft:chiseled_bookshelf", requires_tool=False,
                                  components=burn(4000), states=HORIZONTAL_AXIS),
    # `EXPERIENCE_BLOCK`: `SharedProperties::softMetal` (gold block: 3.0, 6.0), `MapColor.PLANT`,
    # amethyst sounds, `lightLevel(s -> 15)`; its item is `Rarity.UNCOMMON`.
    "experience_block": cube("minecraft:gold_block", 3.0, 6.0, PLANT, "minecraft:amethyst_block",
                             light=15, components={"minecraft:rarity": "uncommon"}),
}

# `pickaxeOnly()`; the casings are `axeOrPickaxe()` (the default) and the cardboard `axeOnly()`.
PICKAXE_ONLY = {"zinc_ore", "deepslate_zinc_ore", "raw_zinc_block", "zinc_block",
                "andesite_alloy_block", "brass_block", "experience_block"}
MINEABLE = {"cardboard_block": ["axe"], "bound_cardboard_block": ["axe"]}


def ore_loot(block, drop):
    """`createSilkTouchDispatchTable`: the ore with Silk Touch, else the raw ore with Fortune's
    ore drops and explosion decay (`ZINC_ORE.loot`), in the form of vanilla's `blocks/gold_ore`."""
    return [{"rolls": 1, "entries": [{"type": "minecraft:alternatives", "children": [
        {"type": "minecraft:item", "condition": "minecraft:tool/can_silk_touch",
         "name": f"create:{block}"},
        {"type": "minecraft:item", "modifier": [
            {"type": "minecraft:apply_bonus", "enchantment": "minecraft:fortune",
             "formula": "minecraft:ore_drops"},
            {"type": "minecraft:explosion_decay"}], "name": drop},
    ]}]}]


SILK = {"type": "minecraft:any_of", "terms": ["minecraft:tool/can_silk_touch"]}

LOOT = {
    "zinc_ore": ore_loot("zinc_ore", "create:raw_zinc"),
    "deepslate_zinc_ore": ore_loot("deepslate_zinc_ore", "create:raw_zinc"),
    # `BOUND_CARDBOARD_BLOCK.loot`: the block with Silk Touch, else a string; and, without Silk
    # Touch, the Block of Cardboard too.
    "bound_cardboard_block": [
        {"rolls": 1, "entries": [{"type": "minecraft:alternatives", "children": [
            {"type": "minecraft:item", "condition": "minecraft:tool/can_silk_touch",
             "name": "create:bound_cardboard_block"},
            {"type": "minecraft:item", "condition": {"type": "minecraft:survives_explosion"},
             "name": "minecraft:string"}]}]},
        {"rolls": 1,
         "condition": {"type": "minecraft:all_of", "terms": [
             {"type": "minecraft:inverted", "term": SILK},
             {"type": "minecraft:survives_explosion"}]},
         "entries": [{"type": "minecraft:item", "name": "create:cardboard_block"}]},
    ],
}

NAMES = {
    # Create's `en_us.json`.
    "item.create.wheat_flour": "Wheat Flour",
    "item.create.dough": "Dough",
    "item.create.cinder_flour": "Cinder Flour",
    "item.create.rose_quartz": "Rose Quartz",
    "item.create.polished_rose_quartz": "Polished Rose Quartz",
    "item.create.powdered_obsidian": "Powdered Obsidian",
    "item.create.sturdy_sheet": "Sturdy Sheet",
    "item.create.propeller": "Propeller",
    "item.create.whisk": "Whisk",
    "item.create.brass_hand": "Brass Hand",
    "item.create.crafter_slot_cover": "Crafter Slot Cover",
    "item.create.electron_tube": "Electron Tube",
    "item.create.transmitter": "Transmitter",
    "item.create.pulp": "Pulp",
    "item.create.cardboard": "Cardboard",
    "item.create.incomplete_precision_mechanism": "Incomplete Precision Mechanism",
    "item.create.unprocessed_obsidian_sheet": "Unprocessed Obsidian Sheet",
    "item.create.incomplete_track": "Incomplete Track",
    "item.create.precision_mechanism": "Precision Mechanism",
    "item.create.blaze_cake_base": "Blaze Cake Base",
    "item.create.blaze_cake": "Blaze Cake",
    "item.create.creative_blaze_cake": "Creative Blaze Cake",
    "item.create.bar_of_chocolate": "Bar of Chocolate",
    "item.create.sweet_roll": "Sweet Roll",
    "item.create.chocolate_glazed_berries": "Chocolate Glazed Berries",
    "item.create.honeyed_apple": "Honeyed Apple",
    "item.create.builders_tea": "Builder's Tea",
    "item.create.raw_zinc": "Raw Zinc",
    "item.create.zinc_ingot": "Zinc Ingot",
    "item.create.brass_ingot": "Brass Ingot",
    "item.create.chromatic_compound": "Chromatic Compound",
    "item.create.shadow_steel": "Shadow Steel",
    "item.create.refined_radiance": "Refined Radiance",
    "item.create.copper_nugget": "Copper Nugget",
    "item.create.zinc_nugget": "Zinc Nugget",
    "item.create.brass_nugget": "Brass Nugget",
    "item.create.experience_nugget": "Nugget of Experience",
    "item.create.copper_sheet": "Copper Sheet",
    "item.create.brass_sheet": "Brass Sheet",
    "item.create.golden_sheet": "Golden Sheet",
    "item.create.crushed_raw_iron": "Crushed Raw Iron",
    "item.create.crushed_raw_gold": "Crushed Raw Gold",
    "item.create.crushed_raw_copper": "Crushed Raw Copper",
    "item.create.crushed_raw_zinc": "Crushed Raw Zinc",
    "block.create.zinc_ore": "Zinc Ore",
    "block.create.deepslate_zinc_ore": "Deepslate Zinc Ore",
    "block.create.raw_zinc_block": "Block of Raw Zinc",
    "block.create.zinc_block": "Block of Zinc",
    "block.create.andesite_alloy_block": "Block of Andesite Alloy",
    "block.create.brass_block": "Block of Brass",
    "block.create.experience_block": "Block of Experience",
    "block.create.cardboard_block": "Block of Cardboard",
    "block.create.bound_cardboard_block": "Bound Block of Cardboard",
    "block.create.andesite_casing": "Andesite Casing",
    "block.create.brass_casing": "Brass Casing",
    "block.create.copper_casing": "Copper Casing",
    "block.create.shadow_steel_casing": "Shadow Casing",
    "block.create.refined_radiance_casing": "Radiant Casing",
}

# --- recipes: Create's datagen (`data/create/recipe/...`) in 26.3's form ---------------------------
# Ingredients are `"id"` / `"#tag"` strings, a result's count is left out when it is 1. Left out:
# Create's own types, other mods' recipes, and the sand paper and rose quartz tile ones (items and
# blocks this module does not declare). The `andesite_alloy` recipe is gen_content.py's.
RECIPES = {
    "crafting/materials/andesite_alloy_block": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "CCC",
            "CCC",
            "CCC"
        ],
        "key": {
            "C": "create:andesite_alloy"
        },
        "result": {
            "id": "create:andesite_alloy_block"
        }
    },
    "crafting/materials/andesite_alloy_from_block": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "create:andesite_alloy_block"
        ],
        "result": {
            "id": "create:andesite_alloy",
            "count": 9
        }
    },
    "crafting/materials/andesite_alloy_from_zinc": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "BA",
            "AB"
        ],
        "key": {
            "A": "minecraft:andesite",
            "B": "#c:nuggets/zinc"
        },
        "result": {
            "id": "create:andesite_alloy"
        }
    },
    "crafting/materials/bound_cardboard_block": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "create:cardboard_block",
            "#c:strings"
        ],
        "result": {
            "id": "create:bound_cardboard_block"
        }
    },
    "crafting/materials/brass_block_from_compacting": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "###",
            "###",
            "###"
        ],
        "key": {
            "#": "#c:ingots/brass"
        },
        "result": {
            "id": "create:brass_block"
        }
    },
    "crafting/materials/brass_ingot_from_compacting": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "###",
            "###",
            "###"
        ],
        "key": {
            "#": "#c:nuggets/brass"
        },
        "result": {
            "id": "create:brass_ingot"
        }
    },
    "crafting/materials/brass_ingot_from_decompacting": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "#c:storage_blocks/brass"
        ],
        "result": {
            "id": "create:brass_ingot",
            "count": 9
        }
    },
    "crafting/materials/brass_nugget_from_decompacting": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "#c:ingots/brass"
        ],
        "result": {
            "id": "create:brass_nugget",
            "count": 9
        }
    },
    "crafting/materials/cardboard_block": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "CC",
            "CC"
        ],
        "key": {
            "C": "create:cardboard"
        },
        "result": {
            "id": "create:cardboard_block"
        }
    },
    "crafting/materials/cardboard_from_block": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "create:cardboard_block"
        ],
        "result": {
            "id": "create:cardboard",
            "count": 4
        }
    },
    "crafting/materials/cardboard_from_bound_block": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "create:bound_cardboard_block"
        ],
        "result": {
            "id": "create:cardboard",
            "count": 4
        }
    },
    "crafting/materials/copper_ingot": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "CCC",
            "CCC",
            "CCC"
        ],
        "key": {
            "C": "#c:nuggets/copper"
        },
        "result": {
            "id": "minecraft:copper_ingot"
        }
    },
    "crafting/materials/copper_nugget": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "#c:ingots/copper"
        ],
        "result": {
            "id": "create:copper_nugget",
            "count": 9
        }
    },
    "crafting/materials/electron_tube": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "L",
            "N"
        ],
        "key": {
            "L": "create:polished_rose_quartz",
            "N": "#c:plates/iron"
        },
        "result": {
            "id": "create:electron_tube"
        }
    },
    "crafting/materials/experience_block": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "CCC",
            "CCC",
            "CCC"
        ],
        "key": {
            "C": "create:experience_nugget"
        },
        "result": {
            "id": "create:experience_block"
        }
    },
    "crafting/materials/experience_nugget_from_block": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "create:experience_block"
        ],
        "result": {
            "id": "create:experience_nugget",
            "count": 9
        }
    },
    "crafting/materials/raw_zinc": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "create:raw_zinc_block"
        ],
        "result": {
            "id": "create:raw_zinc",
            "count": 9
        }
    },
    "crafting/materials/raw_zinc_block": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "CCC",
            "CCC",
            "CCC"
        ],
        "key": {
            "C": "create:raw_zinc"
        },
        "result": {
            "id": "create:raw_zinc_block"
        }
    },
    "crafting/materials/rose_quartz": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "#c:gems/quartz",
            "#c:dusts/redstone",
            "#c:dusts/redstone",
            "#c:dusts/redstone",
            "#c:dusts/redstone",
            "#c:dusts/redstone",
            "#c:dusts/redstone",
            "#c:dusts/redstone",
            "#c:dusts/redstone"
        ],
        "result": {
            "id": "create:rose_quartz"
        }
    },
    "crafting/materials/transmitter": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            " N ",
            "LLL",
            " R "
        ],
        "key": {
            "L": "#c:plates/copper",
            "N": "minecraft:lightning_rod",
            "R": "#c:dusts/redstone"
        },
        "result": {
            "id": "create:transmitter"
        }
    },
    "crafting/materials/zinc_block_from_compacting": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "###",
            "###",
            "###"
        ],
        "key": {
            "#": "#c:ingots/zinc"
        },
        "result": {
            "id": "create:zinc_block"
        }
    },
    "crafting/materials/zinc_ingot_from_compacting": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "###",
            "###",
            "###"
        ],
        "key": {
            "#": "#c:nuggets/zinc"
        },
        "result": {
            "id": "create:zinc_ingot"
        }
    },
    "crafting/materials/zinc_ingot_from_decompacting": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "#c:storage_blocks/zinc"
        ],
        "result": {
            "id": "create:zinc_ingot",
            "count": 9
        }
    },
    "crafting/materials/zinc_nugget_from_decompacting": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "#c:ingots/zinc"
        ],
        "result": {
            "id": "create:zinc_nugget",
            "count": 9
        }
    },
    "crafting/appliances/dough": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "#c:flours/wheat",
            "minecraft:water_bucket"
        ],
        "result": {
            "id": "create:dough"
        }
    },
    "crafting/appliances/chain_from_zinc": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "S",
            "C",
            "S"
        ],
        "key": {
            "C": "#c:ingots/zinc",
            "S": "#c:nuggets/zinc"
        },
        "result": {
            "id": "minecraft:chain"
        }
    },
    "crafting/appliances/slime_ball": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "create:dough",
            "#c:dyes/lime"
        ],
        "result": {
            "id": "minecraft:slime_ball"
        }
    },
    "crafting/appliances/book": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "create:cardboard",
            "minecraft:paper",
            "minecraft:paper",
            "minecraft:paper"
        ],
        "result": {
            "id": "minecraft:book"
        }
    },
    "crafting/appliances/item_frame": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "SSS",
            "SPS",
            "SSS"
        ],
        "key": {
            "P": "create:cardboard",
            "S": "#c:rods/wooden"
        },
        "result": {
            "id": "minecraft:item_frame"
        }
    },
    "crafting/appliances/name_tag": {
        "type": "minecraft:crafting_shapeless",
        "category": "misc",
        "ingredients": [
            "#c:dyes/black",
            "#c:strings",
            "create:cardboard"
        ],
        "result": {
            "id": "minecraft:name_tag"
        }
    },
    "crafting/curiosities/cake": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            " M ",
            "SES",
            " P "
        ],
        "key": {
            "E": "#c:eggs",
            "M": "minecraft:milk_bucket",
            "P": "#c:foods/dough",
            "S": "minecraft:sugar"
        },
        "result": {
            "id": "minecraft:cake"
        }
    },
    "crafting/kinetics/brass_hand": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            " A ",
            "BBB",
            " B "
        ],
        "key": {
            "A": "create:andesite_alloy",
            "B": "#c:plates/brass"
        },
        "result": {
            "id": "create:brass_hand"
        }
    },
    "crafting/kinetics/propeller": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            " S ",
            "SCS",
            " S "
        ],
        "key": {
            "C": "create:andesite_alloy",
            "S": "#c:plates/iron"
        },
        "result": {
            "id": "create:propeller"
        }
    },
    "crafting/kinetics/whisk": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            " C ",
            "SCS",
            "SSS"
        ],
        "key": {
            "C": "create:andesite_alloy",
            "S": "#c:plates/iron"
        },
        "result": {
            "id": "create:whisk"
        }
    },
    "crafting/kinetics/crafter_slot_cover": {
        "type": "minecraft:crafting_shaped",
        "category": "misc",
        "pattern": [
            "AAA"
        ],
        "key": {
            "A": "#c:nuggets/brass"
        },
        "result": {
            "id": "create:crafter_slot_cover"
        }
    },
    "smelting/bread": {
        "type": "minecraft:smelting",
        "category": "food",
        "ingredient": "create:dough",
        "experience": 0.0,
        "cookingtime": 200,
        "result": {
            "id": "minecraft:bread"
        }
    },
    "smoking/bread": {
        "type": "minecraft:smoking",
        "category": "food",
        "ingredient": "create:dough",
        "experience": 0.0,
        "cookingtime": 100,
        "result": {
            "id": "minecraft:bread"
        }
    },
    "campfire_cooking/bread": {
        "type": "minecraft:campfire_cooking",
        "category": "food",
        "ingredient": "create:dough",
        "experience": 0.0,
        "cookingtime": 600,
        "result": {
            "id": "minecraft:bread"
        }
    },
    "smelting/zinc_ingot_from_ore": {
        "type": "minecraft:smelting",
        "category": "misc",
        "ingredient": "#c:ores/zinc",
        "experience": 1.0,
        "cookingtime": 200,
        "result": {
            "id": "create:zinc_ingot"
        }
    },
    "smelting/zinc_ingot_from_raw_ore": {
        "type": "minecraft:smelting",
        "category": "misc",
        "ingredient": "#c:raw_materials/zinc",
        "experience": 0.7,
        "cookingtime": 200,
        "result": {
            "id": "create:zinc_ingot"
        }
    },
    "smelting/zinc_ingot_from_crushed": {
        "type": "minecraft:smelting",
        "category": "misc",
        "ingredient": "create:crushed_raw_zinc",
        "experience": 0.1,
        "cookingtime": 200,
        "result": {
            "id": "create:zinc_ingot"
        }
    },
    "smelting/copper_ingot_from_crushed": {
        "type": "minecraft:smelting",
        "category": "misc",
        "ingredient": "create:crushed_raw_copper",
        "experience": 0.1,
        "cookingtime": 200,
        "result": {
            "id": "minecraft:copper_ingot"
        }
    },
    "smelting/gold_ingot_from_crushed": {
        "type": "minecraft:smelting",
        "category": "misc",
        "ingredient": "create:crushed_raw_gold",
        "experience": 0.1,
        "cookingtime": 200,
        "result": {
            "id": "minecraft:gold_ingot"
        }
    },
    "smelting/iron_ingot_from_crushed": {
        "type": "minecraft:smelting",
        "category": "misc",
        "ingredient": "create:crushed_raw_iron",
        "experience": 0.1,
        "cookingtime": 200,
        "result": {
            "id": "minecraft:iron_ingot"
        }
    },
    "blasting/zinc_ingot_from_ore": {
        "type": "minecraft:blasting",
        "category": "misc",
        "ingredient": "#c:ores/zinc",
        "experience": 1.0,
        "cookingtime": 100,
        "result": {
            "id": "create:zinc_ingot"
        }
    },
    "blasting/zinc_ingot_from_raw_ore": {
        "type": "minecraft:blasting",
        "category": "misc",
        "ingredient": "#c:raw_materials/zinc",
        "experience": 0.7,
        "cookingtime": 100,
        "result": {
            "id": "create:zinc_ingot"
        }
    },
    "blasting/zinc_ingot_from_crushed": {
        "type": "minecraft:blasting",
        "category": "misc",
        "ingredient": "create:crushed_raw_zinc",
        "experience": 0.1,
        "cookingtime": 100,
        "result": {
            "id": "create:zinc_ingot"
        }
    },
    "blasting/copper_ingot_from_crushed": {
        "type": "minecraft:blasting",
        "category": "misc",
        "ingredient": "create:crushed_raw_copper",
        "experience": 0.1,
        "cookingtime": 100,
        "result": {
            "id": "minecraft:copper_ingot"
        }
    },
    "blasting/gold_ingot_from_crushed": {
        "type": "minecraft:blasting",
        "category": "misc",
        "ingredient": "create:crushed_raw_gold",
        "experience": 0.1,
        "cookingtime": 100,
        "result": {
            "id": "minecraft:gold_ingot"
        }
    },
    "blasting/iron_ingot_from_crushed": {
        "type": "minecraft:blasting",
        "category": "misc",
        "ingredient": "create:crushed_raw_iron",
        "experience": 0.1,
        "cookingtime": 100,
        "result": {
            "id": "minecraft:iron_ingot"
        }
    }
}


# --- tags: `AllTags`, `CommonMetal`, the registrate `.tag(...)` calls of `AllItems`/`AllBlocks`, and
# `CreateRegistrateTags` -----------------------------------------------------------------------

STORAGE = {  # `CommonMetal.<metal>.storageBlocks` etc., `AllBlockTags`/`AllItemTags`
    "zinc": "zinc_block", "brass": "brass_block", "raw_zinc": "raw_zinc_block",
    "cardboard": "cardboard_block", "andesite_alloy": "andesite_alloy_block",
}
STORAGE_BLOCKS = ["raw_zinc_block", "zinc_block", "andesite_alloy_block", "brass_block",
                  "cardboard_block", "experience_block"]
ORES = ["zinc_ore", "deepslate_zinc_ore"]


def ids(*names):
    return [f"create:{name}" for name in names]


TAGS = {
    # `CommonMetal.ZINC.ores`, `Tags.Blocks.ORES(_IN_GROUND_*)`, `Tags.Items.ORES(_IN_GROUND_*)`.
    "c:block/ores/zinc": ids(*ORES),
    "c:block/ores": ids(*ORES),
    "c:block/ores_in_ground/stone": ids("zinc_ore"),
    "c:block/ores_in_ground/deepslate": ids("deepslate_zinc_ore"),
    "c:item/ores/zinc": ids(*ORES),
    "c:item/ores": ids(*ORES),
    "c:item/ores_in_ground/stone": ids("zinc_ore"),
    "c:item/ores_in_ground/deepslate": ids("deepslate_zinc_ore"),
    # `tagBlockAndItem(...)` on the storage blocks, `Tags.Blocks/Items.STORAGE_BLOCKS`.
    **{f"c:block/storage_blocks/{metal}": ids(block) for metal, block in STORAGE.items()},
    **{f"c:item/storage_blocks/{metal}": ids(block) for metal, block in STORAGE.items()},
    "c:block/storage_blocks": ids(*STORAGE_BLOCKS),
    "c:item/storage_blocks": ids(*STORAGE_BLOCKS),
    # `RAW_ZINC`: `CommonMetal.ZINC.rawOres`, `Tags.Items.RAW_MATERIALS`.
    "c:item/raw_materials/zinc": ids("raw_zinc"),
    "c:item/raw_materials": ids("raw_zinc"),
    # `CREATE_INGOTS` (the Andesite Alloy is declared in gen_content.py), `CommonMetal` ingots.
    "create:item/create_ingots": ids("andesite_alloy", "zinc_ingot", "brass_ingot"),
    "c:item/ingots": ["#create:create_ingots"],
    "minecraft:item/beacon_payment_items": ["#create:create_ingots"],
    "c:item/ingots/zinc": ids("zinc_ingot"),
    "c:item/ingots/brass": ids("brass_ingot"),
    # NeoForge's copper ingot tag, which the Copper Nugget recipes name.
    "c:item/ingots/copper": ["minecraft:copper_ingot"],
    # Nuggets (`Tags.Items.NUGGETS`). Vanilla has a copper nugget of its own now.
    "c:item/nuggets": ids("copper_nugget", "zinc_nugget", "brass_nugget", "experience_nugget"),
    "c:item/nuggets/copper": ids("copper_nugget") + ["minecraft:copper_nugget"],
    "c:item/nuggets/zinc": ids("zinc_nugget"),
    "c:item/nuggets/brass": ids("brass_nugget"),
    # Sheets: `PLATES`, `CommonMetal` plates, `OBSIDIAN_PLATES`, `CARDBOARD_PLATES`. The Iron Sheet
    # is declared in gen_content.py; its tag membership is Create's.
    "c:item/plates": ids("sturdy_sheet", "cardboard", "copper_sheet", "brass_sheet", "iron_sheet",
                         "golden_sheet"),
    "c:item/plates/obsidian": ids("sturdy_sheet"),
    "c:item/plates/cardboard": ids("cardboard"),
    "c:item/plates/copper": ids("copper_sheet"),
    "c:item/plates/brass": ids("brass_sheet"),
    "c:item/plates/iron": ids("iron_sheet"),
    "c:item/plates/gold": ids("golden_sheet"),
    # `GOLDEN_SHEET` and `CRUSHED_GOLD` are `ItemTags.PIGLIN_LOVED`.
    "minecraft:item/piglin_loved": ids("golden_sheet", "crushed_raw_gold"),
    "create:item/crushed_raw_materials": ids("crushed_raw_iron", "crushed_raw_gold",
                                             "crushed_raw_copper", "crushed_raw_zinc"),
    "c:item/dusts/obsidian": ids("powdered_obsidian"),
    "c:item/flours": ids("wheat_flour"),
    "c:item/flours/wheat": ids("wheat_flour"),
    "c:item/foods/dough": ids("dough"),
    "c:item/foods/dough/wheat": ids("dough"),
    # Foods and drinks: `Tags.Items.FOODS`, `FOODS_BERRY`, `FOODS_FRUIT`, `DRINKS`, `FOODS_CHOCOLATE`.
    "c:item/foods": ids("bar_of_chocolate", "sweet_roll", "chocolate_glazed_berries",
                        "honeyed_apple", "builders_tea"),
    "c:item/foods/chocolate": ids("bar_of_chocolate"),
    "c:item/foods/berry": ids("chocolate_glazed_berries"),
    "c:item/foods/fruit": ids("honeyed_apple"),
    "c:item/drinks": ids("builders_tea"),
    "c:item/drinks/tea": ids("builders_tea"),
    # `UPRIGHT_ON_BELT`: the three cakes and the tea here, then `CreateRegistrateTags`' vanilla members.
    "create:item/upright_on_belt": ids("blaze_cake_base", "blaze_cake", "creative_blaze_cake",
                                       "builders_tea") + [
        "minecraft:glass_bottle", "minecraft:potion", "minecraft:splash_potion",
        "minecraft:lingering_potion", "minecraft:honey_bottle", "minecraft:cake", "minecraft:bowl",
        "minecraft:mushroom_stew", "minecraft:rabbit_stew", "minecraft:beetroot_soup",
        "minecraft:suspicious_stew"],
    # `BuilderTransformers.casing`: `AllBlockTags.CASING` and `AllItemTags.CASING`.
    "create:block/casing": ids("andesite_casing", "brass_casing", "copper_casing",
                               "shadow_steel_casing", "refined_radiance_casing"),
    "create:item/casing": ids("andesite_casing", "brass_casing", "copper_casing",
                              "shadow_steel_casing", "refined_radiance_casing"),
    # Cardboard blocks: `AllBlockTags/AllItemTags.CARDBOARD_STORAGE_BLOCKS` are `c:storage_blocks/cardboard`.
    # Vanilla tags: `BlockTags.NEEDS_IRON_TOOL`, `BlockTags.BEACON_BASE_BLOCKS`.
    "minecraft:block/needs_iron_tool": ids("zinc_ore", "deepslate_zinc_ore", "raw_zinc_block",
                                           "zinc_block", "brass_block"),
    "minecraft:block/beacon_base_blocks": ids("zinc_block", "brass_block", "experience_block"),
    # Vanilla members of the NeoForge convention tags the recipes above name (vanilla has no `c:`).
    "c:item/strings": ["minecraft:string"],
    "c:item/dusts/redstone": ["minecraft:redstone"],
    "c:item/gems/quartz": ["minecraft:quartz"],
    "c:item/rods/wooden": ["minecraft:stick"],
    "c:item/dyes/lime": ["minecraft:lime_dye"],
    "c:item/dyes/black": ["minecraft:black_dye"],
    "c:item/eggs": ["#minecraft:eggs"],
}

# --- creative tab: `AllCreativeModeTabs` lists flat items in `AllItems` order, then the blocks in
# `AllBlocks` order. Left out as Create does: the incomplete items, Chromatic Compound, Shadow Steel,
# Refined Radiance and the two casings of those. The Blaze Cake Base is search-only. ------------------

TAB = [
    "wheat_flour", "dough", "cinder_flour", "rose_quartz", "polished_rose_quartz",
    "powdered_obsidian", "sturdy_sheet", "propeller", "whisk", "brass_hand", "crafter_slot_cover",
    "electron_tube", "transmitter", "pulp", "cardboard", "precision_mechanism", "blaze_cake",
    "creative_blaze_cake", "bar_of_chocolate", "sweet_roll", "chocolate_glazed_berries",
    "honeyed_apple", "builders_tea", "raw_zinc", "zinc_ingot", "brass_ingot", "copper_nugget",
    "zinc_nugget", "brass_nugget", "experience_nugget", "copper_sheet", "brass_sheet",
    "golden_sheet", "crushed_raw_iron", "crushed_raw_gold", "crushed_raw_copper",
    "crushed_raw_zinc",
    "andesite_casing", "brass_casing", "copper_casing", "zinc_ore", "deepslate_zinc_ore",
    "raw_zinc_block", "zinc_block", "andesite_alloy_block", "brass_block", "cardboard_block",
    "bound_cardboard_block", "experience_block",
]

# --- zinc ore in the world: `AllConfiguredFeatures`, `AllPlacedFeatures`, `AllBiomeModifiers` -------
# 26.3's forms (flat feature, block states as ids), loaded through NeoForge's biome modifier.

FILES = {
    # `new OreConfiguration(zincTargetStates, 12)`: stone and deepslate ore replaceables.
    "data/create/worldgen/feature/zinc_ore.json": {
        "type": "minecraft:ore",
        "discard_chance_on_air_exposure": 0.0,
        "size": 12,
        "targets": [
            {"state": "create:zinc_ore",
             "target": {"predicate_type": "minecraft:tag_match",
                        "tag": "minecraft:stone_ore_replaceables"}},
            {"state": "create:deepslate_zinc_ore",
             "target": {"predicate_type": "minecraft:tag_match",
                        "tag": "minecraft:deepslate_ore_replaceables"}},
        ],
    },
    # `placement(CountPlacement.of(8), -63, 70)`: count, in_square, height_range uniform; Create's
    # `config_filter` (a switch in its config) is left out.
    "data/create/worldgen/placed_feature/zinc_ore.json": {
        "feature": "create:zinc_ore",
        "placement": [
            {"type": "minecraft:count", "count": 8},
            {"type": "minecraft:in_square"},
            {"type": "minecraft:height_range", "height": {
                "type": "minecraft:uniform",
                "max_inclusive": {"absolute": 70}, "min_inclusive": {"absolute": -63}}},
        ],
    },
    # `addOre(isOverworld, zincOre)`: `Decoration.UNDERGROUND_ORES`.
    "data/create/neoforge/biome_modifier/zinc_ore.json": {
        "type": "neoforge:add_features",
        "biomes": "#minecraft:is_overworld",
        "features": "create:zinc_ore",
        "step": "underground_ores",
    },
}
