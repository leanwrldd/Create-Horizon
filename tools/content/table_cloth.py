"""The Table Cloths and the Shopping List: `create:<colour>_table_cloth`, `create:andesite_table_cloth`, `create:brass_table_cloth`,
`create:copper_table_cloth` and `create:shopping_list`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.TABLE_CLOTHS` (sixteen `DyedBlockList` blocks, `BuilderTransformers.tableCloth(name,
() -> Blocks.BLACK_CARPET, true)`, `MapColor` the dye's, tags `table_cloths` and `combination_step_sound_blocks`, recipes
`crafting/logistics/<colour>_table_cloth` (a wool of the dye and an Andesite Alloy make two) and `..._from_other_table_cloth`) and the
three covers (`ANDESITE_TABLE_CLOTH` from `SharedProperties.stone` and `MapColor.STONE`, `BRASS_TABLE_CLOTH` from `softMetal` and
`TERRACOTTA_YELLOW`, `COPPER_TABLE_CLOTH` from `copperMetal`; each `requiresCorrectToolForDrops`, `pickaxeOnly`, tag
`inside_step_sound_blocks`, stonecut from Andesite Alloy or a brass or copper ingot, two for one), `TableClothBlock` (`entity`, which
isn't kept here: every cloth has a block entity), `AllShapes.TABLE_CLOTH` (`shape(-1, -9, -1, 17, 1, 17)`, the outline and what a click
hits) and `TABLE_CLOTH_OCCLUSION` (`shape(0, 0, 0, 16, 1, 16)`, what collides), `AllItems.SHOPPING_LIST` (`stacksTo(1)`). The `_clear`
recipes aren't declared: a cloth carries no order here. The behaviour is `mod/src/machines/table_cloth.rs`.
"""
from gen_content import block

# `DyeColor`: (name, `getMapColor().id`).
DYES = [("white", 8), ("orange", 15), ("magenta", 16), ("light_blue", 17), ("yellow", 18), ("lime", 19), ("pink", 20), ("gray", 21),
        ("light_gray", 22), ("cyan", 23), ("purple", 24), ("blue", 25), ("brown", 26), ("green", 27), ("red", 28), ("black", 29)]
# `Blocks.BLACK_CARPET`: `strength(0.1F)`, `SoundType.WOOL`.
CARPET = {"hardness": 0.1, "explosion_resistance": 0.1, "requires_tool": False, "sounds": "minecraft:white_wool"}
STONE_MAP, TERRACOTTA_YELLOW, ORANGE = 11, 40, 15
COVERS = [
    # (name, hardness, resistance, sounds, map colour, the stonecutting's ingredient)
    ("andesite", 1.5, 6.0, "minecraft:andesite", STONE_MAP, "create:andesite_alloy"),
    ("brass", 3.0, 6.0, "minecraft:gold_block", TERRACOTTA_YELLOW, "#c:ingots/brass"),
    ("copper", 3.0, 6.0, "minecraft:copper_block", ORANGE, "#c:ingots/copper"),
]
# The outline reaches a texel past the block and 9 below (the click's shape); the cloth itself is one texel thick.
OUTLINE = {"default": [[-1 / 16, -9 / 16, -1 / 16, 17 / 16, 1 / 16, 17 / 16]]}
COLLISION = {"default": [[0, 0, 0, 1, 1 / 16, 1]]}

BLOCKS = {}
NAMES = {}
RECIPES = {}
TAGS = {"create:item/table_cloths": [], "create:item/dyed_table_cloths": [], "create:block/table_cloths": []}
PICKAXE_ONLY = set()
# A dyed cloth copies the carpet, which no tool tag lists.
MINEABLE = {}
TAB = []


def _cloth(name, base, map_color):
    value = block(None, None, base, map_color, OUTLINE, sounds=base["sounds"], render_shape="MODEL")
    value["collision_shapes"] = COLLISION
    BLOCKS[name] = value
    TAGS["create:item/table_cloths"].append(f"create:{name}")
    TAGS["create:block/table_cloths"].append(f"create:{name}")
    TAB.append(name)


for colour, map_color in DYES:
    name = f"{colour}_table_cloth"
    _cloth(name, CARPET, map_color)
    TAGS["create:item/dyed_table_cloths"].append(f"create:{name}")
    TAGS[f"c:item/dyes/{colour}"] = [f"minecraft:{colour}_dye"]
    MINEABLE[name] = []
    NAMES[f"block.create.{name}"] = colour.replace("_", " ").title() + " Table Cloth"
    RECIPES[f"crafting/logistics/{name}"] = {
        "type": "minecraft:crafting_shapeless", "category": "building",
        "ingredients": [f"minecraft:{colour}_wool", "create:andesite_alloy"],
        "result": {"id": f"create:{name}", "count": 2},
    }
    RECIPES[f"crafting/logistics/{name}_from_other_table_cloth"] = {
        "type": "minecraft:crafting_shapeless", "category": "building",
        "ingredients": [f"#c:dyes/{colour}", "#create:dyed_table_cloths"],
        "result": {"id": f"create:{name}", "count": 1},
    }

for cover, hardness, resistance, sounds, map_color, ingredient in COVERS:
    name = f"{cover}_table_cloth"
    base = {"hardness": hardness, "explosion_resistance": resistance, "requires_tool": True, "sounds": sounds}
    _cloth(name, base, map_color)
    PICKAXE_ONLY.add(name)
    NAMES[f"block.create.{name}"] = f"{cover.title()} Table Cover"
    label = ingredient.removeprefix("#").split(":", 1)[-1].replace("/", "_")
    RECIPES[f"{name}_from_{label}_stonecutting"] = {
        "type": "minecraft:stonecutting", "ingredient": ingredient, "result": {"id": f"create:{name}", "count": 2}}

# `AllItems.SHOPPING_LIST`: `stacksTo(1)`.
ITEMS = {"shopping_list": {"template": "minecraft:iron_ingot", "max_stack_size": 1}}
NAMES["item.create.shopping_list"] = "Shopping List"
