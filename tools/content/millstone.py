"""The Millstone: `create:millstone` and every `create:milling` recipe of Create's datagen.

Create: `AllBlocks.MILLSTONE` (`SharedProperties.stone`, `MapColor.METAL`, `pickaxeOnly`),
`AllShapes.MILLSTONE`, `recipe/crafting/kinetics/millstone`, `recipe/milling/*`.
"""

import importlib.util
from pathlib import Path

from gen_content import ANDESITE, METAL, block


def _data(name, const):
    """A generated recipe table beside this file (the plugin loader skips `_` files)."""
    path = Path(__file__).with_name(name)
    spec = importlib.util.spec_from_file_location(f"create_content_{path.stem}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return getattr(module, const)


# `AllShapes.MILLSTONE = shape(0, 0, 0, 16, 6, 16).add(2, 6, 2, 14, 16, 14)`, in blocks. The
# casing is in the chunk mesh (`create:block/millstone/block`); the block entity draws the cog.
BLOCKS = {
    "millstone": block({}, {}, ANDESITE, METAL,
                       {"default": [[0, 0, 0, 1, 6 / 16, 1],
                                    [2 / 16, 6 / 16, 2 / 16, 14 / 16, 1, 14 / 16]]},
                       render_shape="MODEL"),
}
PICKAXE_ONLY = {"millstone"}
NAMES = {"block.create.millstone": "Millstone"}
TAB = ["millstone"]
MODELS = ["block/millstone/inner"]  # `AllPartialModels.MILLSTONE_COG`

RECIPES = {
    **_data("_millstone_recipes.py", "MILLING"),
    # `recipe/crafting/kinetics/millstone`.
    "millstone": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["C", "S", "I"],
        "key": {"C": "create:cogwheel", "S": "create:andesite_casing", "I": "#c:stones"},
        "result": {"id": "create:millstone"},
    },
}
# What NeoForge's `c:stones` holds of vanilla's (`Tags.Items.STONES`).
TAGS = {"c:item/stones": ["minecraft:stone", "minecraft:granite", "minecraft:diorite",
                          "minecraft:andesite", "minecraft:deepslate", "minecraft:tuff"]}
