"""The Train Observer: `create:track_observer` (module `track_observer`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.TRACK_OBSERVER` (`TrackObserverBlock`; `SharedProperties.softMetal`,
`MapColor.PODZOL`, `noOcclusion`, `SoundType.NETHERITE_BLOCK`, `pickaxeOnly`, a `TrackTargetingBlockItem` of
`EdgePointType.OBSERVER`, the item model of the block), `TrackObserverBlock` (`powered`; no `getShape`, so a full
block; a signal source of 15 while powered), `recipe/crafting/kinetics/track_observer` and
`track_observer_from_other_plates` (Create's datagen files), `logistics.train_observer.cargo_filter`.

The block's model is Create's blockstate from the player's jar (`block` and `block_powered`). The cargo filter is a
value box on the top face (`createFilter`); the display source (`OBSERVED_TRAIN_NAME`) isn't declared.
"""

from gen_content import PODZOL, block

# `SharedProperties.softMetal`: a gold block (3.0 / 6.0), with `SoundType.NETHERITE_BLOCK`.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:netherite_block"}

BLOCKS = {
    "track_observer": block({"powered": {"type": "boolean"}}, {"powered": "false"}, SOFT_METAL, PODZOL,
                            render_shape="MODEL"),
}
BLOCKS["track_observer"]["properties"]["occludes"] = False  # `noOcclusion`

TAB = ["track_observer"]

NAMES = {
    "block.create.track_observer": "Train Observer",
    "create.logistics.train_observer.cargo_filter": "Cargo Filter",
}

RECIPES = {
    # `data/create/recipe/crafting/kinetics/track_observer.json`.
    "crafting/kinetics/track_observer": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:railway_casing", "#minecraft:wooden_pressure_plates"],
        "result": {"id": "create:track_observer", "count": 2},
    },
    # `track_observer_from_other_plates.json`.
    "crafting/kinetics/track_observer_from_other_plates": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": [
            "create:railway_casing",
            ["minecraft:stone_pressure_plate", "minecraft:polished_blackstone_pressure_plate",
             "minecraft:heavy_weighted_pressure_plate", "minecraft:light_weighted_pressure_plate"],
        ],
        "result": {"id": "create:track_observer", "count": 2},
    },
}

PICKAXE_ONLY = {"track_observer"}
