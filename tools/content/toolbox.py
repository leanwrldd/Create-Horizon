"""The Toolbox (module `toolbox`): `create:<colour>_toolbox` for the 16 dye colours.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.TOOLBOXES` (`SharedProperties.wooden`, `SoundType.WOOD`,
`mapColor(colour)`, `ToolboxBlock`: `facing` and `waterlogged`; its item is an `UncontainableBlockItem`),
`AllShapes.TOOLBOX` (`shape(1, 0, 4, 15, 9, 12).forHorizontal(NORTH)`), the tags `create:toolboxes`,
`crafting/curiosities/brown_toolbox` and `toolbox_dyeing` (a custom recipe; `minecraft:crafting_transmute`
does the same here and keeps the contents). The body is Create's `block/<colour>_toolbox` model in the chunk
mesh; the block entity draws the lid and the drawers. Behaviour: `native/api/src/machines/toolbox.rs`.
"""
from gen_content import HORIZONTAL_FACING_STATE, WOOD_SOUND, WOODEN, block

# `DyeColor`: `(name, MapColor id of getMapColor())`.
COLOURS = [("white", 8), ("orange", 15), ("magenta", 16), ("light_blue", 17), ("yellow", 18),
           ("lime", 19), ("pink", 20), ("gray", 21), ("light_gray", 22), ("cyan", 23),
           ("purple", 24), ("blue", 25), ("brown", 26), ("green", 27), ("red", 28), ("black", 29)]

# `shape(1, 0, 4, 15, 9, 12)`, facing north: wide across x; turned for east and west.
_SHAPES = {"property": "facing", "values": {
    "north": [[1 / 16, 0, 4 / 16, 15 / 16, 9 / 16, 12 / 16]],
    "south": [[1 / 16, 0, 4 / 16, 15 / 16, 9 / 16, 12 / 16]],
    "east": [[4 / 16, 0, 1 / 16, 12 / 16, 9 / 16, 15 / 16]],
    "west": [[4 / 16, 0, 1 / 16, 12 / 16, 9 / 16, 15 / 16]],
}}

BLOCKS = {
    f"{name}_toolbox": block(HORIZONTAL_FACING_STATE, {"facing": "north"}, WOODEN, colour, _SHAPES,
                             sounds=WOOD_SOUND, render_shape="MODEL")
    for name, colour in COLOURS
}
for _name in BLOCKS:
    BLOCKS[_name]["properties"]["occludes"] = False

NAMES = {
    **{f"block.create.{name}_toolbox": f"{name.replace('_', ' ').title()} Toolbox" for name, _ in COLOURS},
    "tag.block.create.toolboxes": "Toolboxes",
    "tag.item.create.toolboxes": "Toolboxes",
}

# The lid and the drawer (`AllPartialModels.TOOLBOX_LIDS`, `TOOLBOX_DRAWER`) the block entity draws.
MODELS = ["block/toolbox/drawer"] + [f"block/toolbox/lid/{name}" for name, _ in COLOURS]

RECIPES = {
    # `crafting/curiosities/brown_toolbox`.
    "crafting/curiosities/brown_toolbox": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" C ", "SWS", " L "],
        "key": {"C": "create:cogwheel", "L": "#c:leathers", "S": "#c:plates/gold", "W": "#c:chests/wooden"},
        "result": {"id": "create:brown_toolbox", "count": 1},
    },
    # `ToolboxDyeingRecipe`: one toolbox and one dye make the toolbox of that colour, keeping what it holds.
    **{
        f"crafting/curiosities/{name}_toolbox_dyeing": {
            "type": "minecraft:crafting_transmute", "category": "misc", "group": "toolbox_dyeing",
            "input": "#create:toolboxes", "material": f"minecraft:{name}_dye",
            "result": {"id": f"create:{name}_toolbox"},
        }
        for name, _ in COLOURS
    },
}

TAGS = {
    # `AllBlockTags.TOOLBOXES`, `AllItemTags.TOOLBOXES`.
    "create:block/toolboxes": [f"create:{name}_toolbox" for name, _ in COLOURS],
    "create:item/toolboxes": [f"create:{name}_toolbox" for name, _ in COLOURS],
    # NeoForge's `c:leathers` and `c:chests/wooden` (`Tags.Items.LEATHERS`, `CHESTS_WOODEN`).
    "c:item/leathers": ["minecraft:leather"],
    "c:item/chests/wooden": ["minecraft:chest", "minecraft:trapped_chest"],
}

# `DyedBlockList` order: the colours as `DyeColor` lists them.
TAB = [f"{name}_toolbox" for name, _ in COLOURS]
