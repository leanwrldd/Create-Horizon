"""The Deployer: `create:deployer`, the recipes it applies (`create:deploying`,
`create:item_application`) and the tags and damage type it uses.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.DEPLOYER` (`SharedProperties.stone`,
`MapColor.PODZOL`, axe or pickaxe, `CStress.setImpact(4.0)`), `DeployerBlock` (`facing`, from
`DirectionalKineticBlock`, and `axis_along_first`, from `DirectionalAxisKineticBlock`;
`waterlogged` is left out, as elsewhere; `AllShapes.DEPLOYER_INTERACTION` for the outline and
`AllShapes.CASING_12PX` for the collision), `AllPartialModels.DEPLOYER_POLE`,
`DEPLOYER_HAND_POINTING`, `DEPLOYER_HAND_PUNCHING`, `DEPLOYER_HAND_HOLDING`,
`recipe/crafting/kinetics/deployer`, and the datagen's `recipe/deploying/*` and
`recipe/item_application/*` (tables made by `_deployer_recipes_gen.py`).

Also here, because the held item selects them (`DeployerBlockEntity.getRecipe`): the
`recipe/sandpaper_polishing/*` recipes (the Rose Quartz) and the `recipe/sequenced_assembly/*` ones
(the Precision Mechanism and the Sturdy Sheet), which the Press, Saw and Spout read too.

Left out: `item_application/railway_casing` (no Railway Casing yet) and `sequenced_assembly/track`
(no Track or Sleepers yet). The tags
`c:stripped_logs` and `c:stripped_woods` are NeoForge's, which the casing recipes name: they hold
the stripped logs, stems and woods of the game's own registry.
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


_STATES = {**FACING_STATE, "axis_along_first": {"type": "boolean"}}

# `AllShapes.CASING_12PX = shape(0, 0, 0, 16, 12, 16).forDirectional()` and
# `SIX_VOXEL_POLE = shape(5, 0, 5, 11, 16, 11).forAxis()`, written for facing up.
_CASING = [0, 0, 0, 1, 12 / 16, 1]
_POLE = [5 / 16, 0, 5 / 16, 11 / 16, 1, 11 / 16]


def _deployer():
    declared = block(_STATES, {"facing": "north", "axis_along_first": "false"}, ANDESITE, PODZOL,
                     facing_shapes([_CASING, _POLE]), render_shape="MODEL")
    # `getCollisionShape` is the casing alone: the pole is only for the cursor.
    declared["collision_shapes"] = facing_shapes([_CASING])
    return declared


BLOCKS = {
    # The casing is in the chunk mesh (`create:block/deployer/horizontal` or `vertical`, by the
    # blockstate's facing and `axis_along_first`); the block entity draws the shaft, the pole, the
    # hand and the item in it.
    "deployer": _deployer(),
}
# `axeOrPickaxe()`: the default tags (both tools).

# What the block entity draws besides the block's model.
MODELS = ["block/deployer/pole", "block/deployer/hand_pointing", "block/deployer/hand_punching",
          "block/deployer/hand_holding"]

TAB = ["deployer"]

NAMES = {
    "block.create.deployer": "Deployer",
    # `create.block.deployer.damage_source_name` is "a rogue Deployer": what the fake player is
    # called in the death message of a player attack, `death.attack.player`.
    "death.attack.create.deployer": "%1$s was slain by a rogue Deployer",
}

RECIPES = {
    # `recipe/crafting/kinetics/deployer`, in Horizon's 26.3 form.
    "crafting/kinetics/deployer": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "create:electron_tube", "C": "create:andesite_casing",
                "I": "create:brass_hand"},
        "result": {"id": "create:deployer", "count": 1},
    },
    **_data("_deployer_recipes.py", "DEPLOYING"),
    **_data("_deployer_recipes.py", "ITEM_APPLICATION"),
    **_data("_deployer_recipes.py", "SANDPAPER_POLISHING"),
    **_data("_deployer_recipes.py", "SEQUENCED_ASSEMBLY"),
}

# NeoForge's `c:stripped_logs` and `c:stripped_woods`, which the casing recipes name, as the game's
# registry has them.
_LOGS = ["acacia_log", "bamboo_block", "birch_log", "cherry_log", "crimson_stem", "dark_oak_log",
         "jungle_log", "mangrove_log", "oak_log", "pale_oak_log", "poplar_log", "spruce_log",
         "warped_stem"]
_WOODS = ["acacia_wood", "birch_wood", "cherry_wood", "crimson_hyphae", "dark_oak_wood",
          "jungle_wood", "mangrove_wood", "oak_wood", "pale_oak_wood", "poplar_wood",
          "spruce_wood", "warped_hyphae"]
TAGS = {
    "c:item/stripped_logs": [f"minecraft:stripped_{name}" for name in _LOGS],
    "c:item/stripped_woods": [f"minecraft:stripped_{name}" for name in _WOODS],
}

# The damage type of a Deployer's attack. Create has no file for it (its fake player attacks as a
# player: `DamageSource` `minecraft:player_attack`, which names an attacker the Deployer here has
# not got); this is `player_attack` with a death message of its own, `death.attack.create.deployer`.
FILES = {
    "data/create/damage_type/deployer.json": {
        "exhaustion": 0.1, "message_id": "create.deployer",
        "scaling": "when_caused_by_living_non_player"},
}
