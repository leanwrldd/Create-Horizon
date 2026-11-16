"""The Mechanical Crafter: `create:mechanical_crafter` and its recipes (module `crafter`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.MECHANICAL_CRAFTER` (`SharedProperties.softMetal`,
`MapColor.TERRACOTTA_YELLOW`, `noOcclusion`, `axeOrPickaxe`, `HorizontalKineticBlock` with a
`POINTING` property, `CStress.setImpact(2.0)`, `CrafterCTBehaviour`), `AllPartialModels`
`MECHANICAL_CRAFTER_LID`, `_ARROW`, `_BELT_FRAME` (`block/mechanical_crafter/belt`) and `_BELT`
(`block/mechanical_crafter/belt_animated`), `recipe/crafting/kinetics/mechanical_crafter` and
`crafter_slot_cover` (declared with the other materials), and `recipe/mechanical_crafting/*`.

The item `create:crafter_slot_cover` is in `materials.py`. The `create:mechanical_crafting`
recipe of the Crushing Wheel is in `crushing_wheel.py`; Create's other three (Extendo Grip, Potato
Cannon, Wand of Symmetry) make items this mod doesn't have. The vanilla crafting recipes the
crafter also accepts are baked in `_crafter_recipes.py` (see `_crafter_recipes_gen.py`).
The behaviour is `native/api/src/machines/crafter*.rs`.
"""

import importlib.util
from pathlib import Path

from gen_content import HORIZONTAL_FACING_STATE, block

TERRACOTTA_YELLOW = 40  # MapColor.TERRACOTTA_YELLOW
# `SharedProperties.softMetal`: a gold block.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:gold_block"}


def _data(name, const):
    """A generated recipe table beside this file (the plugin loader skips `_` files)."""
    path = Path(__file__).with_name(name)
    spec = importlib.util.spec_from_file_location(f"create_content_{path.stem}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return getattr(module, const)


BLOCKS = {
    # The casing is in the chunk mesh (`block/mechanical_crafter/block`, turned by the
    # blockstate); the block entity draws the cog, the lid, the arrow or the belt, and the items.
    # No shape of its own: a full block.
    "mechanical_crafter": block(
        {**HORIZONTAL_FACING_STATE,
         "pointing": {"type": "enum", "values": ["up", "left", "down", "right"]}},
        {"facing": "north", "pointing": "up"}, SOFT_METAL, TERRACOTTA_YELLOW,
        render_shape="MODEL"),
}
# `axeOrPickaxe()`: the default tags (both tools).

# What the block entity draws besides the block's model.
MODELS = ["block/mechanical_crafter/lid", "block/mechanical_crafter/arrow",
          "block/mechanical_crafter/belt", "block/mechanical_crafter/belt_animated",
          "block/cogwheel_shaftless"]

TAB = ["mechanical_crafter"]

NAMES = {"block.create.mechanical_crafter": "Mechanical Crafter"}

# What NeoForge's `c:player_workstations/crafting_tables` holds of vanilla's.
TAGS = {"c:item/player_workstations/crafting_tables": ["minecraft:crafting_table"]}

RECIPES = {
    # `recipe/crafting/kinetics/mechanical_crafter`, in Horizon's 26.3 form.
    "crafting/kinetics/mechanical_crafter": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "R"],
        "key": {"B": "create:electron_tube", "C": "create:brass_casing",
                "R": "#c:player_workstations/crafting_tables"},
        "result": {"id": "create:mechanical_crafter", "count": 3},
    },
    **_data("_crafter_recipes.py", "BAKED"),
}
