"""The Mechanical Harvester: `create:mechanical_harvester` (module `harvester`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.MECHANICAL_HARVESTER` (`SharedProperties.stone`,
`MapColor.METAL`, `forceSolidOn`, `axeOrPickaxe`, `horizontalBlockProvider(true)`, cutout layer, item tagged
`create:contraption_controlled`), `HarvesterBlock` (an `AttachedActorBlock`: `facing`, `waterlogged` left out
as elsewhere), `AllShapes.HARVESTER_BASE` (`shape(0, 2, 0, 16, 14, 3).forDirectional(SOUTH)`),
`recipe/crafting/kinetics/mechanical_harvester`, `AllBlockTags.NON_HARVESTABLE`.

The block does nothing on its own: it works on a moving structure
(`native/api/src/machines/actor_tools.rs`). The stationary block draws its blade
(`HarvesterRenderer`, `AllPartialModels.HARVESTER_BLADE`).
"""

from gen_content import ANDESITE, HORIZONTAL_FACING_STATE, METAL, block

# `HARVESTER_BASE`, in blocks: the back wall, 3 pixels thick, on the side opposite the facing.
SHAPE = {"property": "facing", "values": {
    "north": [[0, 2 / 16, 13 / 16, 1, 14 / 16, 1]],
    "south": [[0, 2 / 16, 0, 1, 14 / 16, 3 / 16]],
    "east": [[0, 2 / 16, 0, 3 / 16, 14 / 16, 1]],
    "west": [[13 / 16, 2 / 16, 0, 1, 14 / 16, 1]],
}}

BLOCKS = {
    "mechanical_harvester": block(HORIZONTAL_FACING_STATE, {"facing": "north"}, ANDESITE, METAL, SHAPE,
                                  render_shape="MODEL"),
}

# What the block entity draws besides the block's model.
MODELS = ["block/mechanical_harvester/blade"]

TAB = ["mechanical_harvester"]

NAMES = {"block.create.mechanical_harvester": "Mechanical Harvester"}

TAGS = {
    "create:item/contraption_controlled": ["create:mechanical_harvester"],
    # `data/create/tags/block/non_harvestable.json`.
    "create:block/non_harvestable": ["minecraft:fire"],
    # `data/create/tags/block/sugar_cane_variants.json`.
    "create:block/sugar_cane_variants": ["minecraft:sugar_cane"],
}

RECIPES = {
    # `recipe/crafting/kinetics/mechanical_harvester`, in Horizon's 26.3 form.
    "crafting/kinetics/mechanical_harvester": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["AIA", "AIA", " C "],
        "key": {"A": "create:andesite_alloy", "C": "create:andesite_casing", "I": "#c:plates/iron"},
        "result": {"id": "create:mechanical_harvester", "count": 1},
    },
}
