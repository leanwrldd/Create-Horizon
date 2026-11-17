"""The Postboxes: `create:<colour>_postbox` for the sixteen dye colours.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.PACKAGE_POSTBOXES` (`SharedProperties.wooden`, the dye's
`MapColor`, `axeOnly`, tag `create:postboxes`, lang "<Colour> Postbox"), `PostboxBlock` (`facing`, `open`;
waterlogging left out as elsewhere), `AllShapes.POSTBOX` (`shape(2, 0, 0, 14, 14, 16).forHorizontal(SOUTH)`),
the blockstates Create's datagen writes (`<colour>_postbox_closed` and `_open`), the flag partial model
`block/package_postbox/flag`, and the recipes `crafting/logistics/<colour>_postbox` and
`<colour>_postbox_from_other_postbox`. The behaviour is `native/api/src/machines/postbox.rs`.
"""
from gen_content import WOODEN, block

# `DyeColor.getMapColor`: snow, then the colour ids 15 to 29.
COLOURS = {
    "white": 8, "orange": 15, "magenta": 16, "light_blue": 17, "yellow": 18, "lime": 19, "pink": 20,
    "gray": 21, "light_gray": 22, "cyan": 23, "purple": 24, "blue": 25, "brown": 26, "green": 27,
    "red": 28, "black": 29,
}

# Along the facing the box is the full depth; across it 2 to 14.
_SHAPES = {
    "north": [[2 / 16, 0, 0, 14 / 16, 14 / 16, 1]], "south": [[2 / 16, 0, 0, 14 / 16, 14 / 16, 1]],
    "west": [[0, 0, 2 / 16, 1, 14 / 16, 14 / 16]], "east": [[0, 0, 2 / 16, 1, 14 / 16, 14 / 16]],
}
_STATES = {"facing": {"type": "enum", "values": ["north", "south", "west", "east"]},
           "open": {"type": "boolean"}}

BLOCKS = {
    f"{name}_postbox": block(_STATES, {"facing": "north", "open": "false"}, WOODEN, colour,
                             {"property": "facing", "values": _SHAPES}, render_shape="MODEL")
    for name, colour in COLOURS.items()
}
MINEABLE = {f"{name}_postbox": ["axe"] for name in COLOURS}

MODELS = ["block/package_postbox/flag"]

NAMES = {f"block.create.{name}_postbox": f"{name.replace('_', ' ').title()} Postbox" for name in COLOURS}
TAB = [f"{name}_postbox" for name in COLOURS]
PALETTES_TAB = []

TAGS = {
    "create:block/postboxes": [f"create:{name}_postbox" for name in COLOURS],
    "create:item/postboxes": [f"create:{name}_postbox" for name in COLOURS],
}

RECIPES = {}
for _name in COLOURS:
    RECIPES[f"crafting/logistics/{_name}_postbox"] = {
        "type": "minecraft:crafting_shaped", "category": "building",
        "pattern": ["D", "B", "A"],
        "key": {"A": "create:andesite_alloy", "B": "minecraft:barrel", "D": f"#c:dyes/{_name}"},
        "result": {"id": f"create:{_name}_postbox", "count": 1},
    }
    RECIPES[f"crafting/logistics/{_name}_postbox_from_other_postbox"] = {
        "type": "minecraft:crafting_shapeless", "category": "building",
        "ingredients": [f"#c:dyes/{_name}", "#create:postboxes"],
        "result": {"id": f"create:{_name}_postbox", "count": 1},
    }
