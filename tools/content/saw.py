"""The Mechanical Saw: `create:mechanical_saw` and every `create:cutting` recipe it cuts with.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.MECHANICAL_SAW` (`SharedProperties.stone`,
`MapColor.PODZOL`, axe or pickaxe, `CStress.setImpact(4.0)`), `SawBlock` (`facing`, from
`DirectionalKineticBlock`, `axis_along_first`, from `DirectionalAxisKineticBlock`, and `flipped`;
`AllShapes.CASING_12PX`), `recipe/crafting/kinetics/mechanical_saw`, `recipe/cutting/*`,
`data/create/damage_type/mechanical_saw`, and the block tags `TreeCutter` and `isSawable` read
(`CreateRegistrateTags`). The recipe tables are in `_saw_recipes.py`, made by
`_saw_recipes_gen.py`.
"""

import importlib.util
from pathlib import Path

from gen_content import ANDESITE, FACING_STATE, PODZOL, block, facing_shapes


def _data(name, const):
    """A generated recipe table beside this file (the plugin loader skips `_` files)."""
    path = Path(__file__).with_name(name)
    spec = importlib.util.spec_from_file_location(f"create_content_{path.stem}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return getattr(module, const)


_STATES = {
    **FACING_STATE,
    "axis_along_first": {"type": "boolean"},
    "flipped": {"type": "boolean"},
}

BLOCKS = {
    # `AllShapes.CASING_12PX`, as the drill's. The casing is in the chunk mesh
    # (`create:block/mechanical_saw/horizontal` or `vertical`); the block entity draws the blade
    # and the shaft.
    "mechanical_saw": block(
        _STATES, {"facing": "north", "axis_along_first": "false", "flipped": "false"},
        ANDESITE, PODZOL, facing_shapes([[0, 0, 0, 1, 12 / 16, 1]]), render_shape="MODEL"),
}
# `axeOrPickaxe()`: the default tags (both tools).

# `AllPartialModels.SAW_BLADE_*`.
MODELS = [f"block/mechanical_saw/blade_{kind}_{state}"
          for kind in ("horizontal", "vertical") for state in ("active", "inactive", "reversed")]

TAB = ["mechanical_saw"]

NAMES = {
    "block.create.mechanical_saw": "Mechanical Saw",
    "death.attack.create.mechanical_saw": "%1$s got cut in half by a Mechanical Saw",
    "death.attack.create.mechanical_saw.player": "%1$s was thrown into a Saw by %2$s",
}

RECIPES = {
    **_data("_saw_recipes.py", "CUTTING"),
    # RuntimeDataGenerator.cuttingRecipes: written at load time in Create, baked here.
    **_data("_saw_recipes.py", "WOOD"),
    # `recipe/crafting/kinetics/mechanical_saw`, in Horizon's 26.3 form.
    "crafting/kinetics/mechanical_saw": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" A ", "AIA", " C "],
        "key": {"A": "#c:plates/iron", "C": "create:andesite_casing", "I": "#c:ingots/iron"},
        "result": {"id": "create:mechanical_saw", "count": 1},
    },
}

# The tags the saw reads (`CreateRegistrateTags`, with what other mods add left out):
# `AllBlockTags.ROOTS`, `TREE_ATTACHMENTS` and `SOLID_CROPS`. `create:non_breakable` and
# `create:slimy_logs` have nothing of vanilla's in them.
TAGS = {
    "create:block/roots": ["minecraft:mangrove_roots"],
    "create:block/tree_attachments": [
        "minecraft:bee_nest", "minecraft:cocoa", "minecraft:mangrove_propagule",
        "minecraft:moss_carpet", "minecraft:shroomlight", "minecraft:vine"],
    "create:block/solid_crops": ["minecraft:pumpkin", "minecraft:melon"],
}

# `data/create/damage_type/mechanical_saw.json` (`CreateDamageSources.saw`).
FILES = {
    "data/create/damage_type/mechanical_saw.json": {
        "exhaustion": 0.0, "message_id": "create.mechanical_saw",
        "scaling": "when_caused_by_living_non_player"},
}
