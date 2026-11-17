"""The Mechanical Plough: `create:mechanical_plough` (the placement the three attached actors share is in
module `harvester`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.MECHANICAL_PLOUGH` (`SharedProperties.stone`,
`MapColor.COLOR_GRAY`, `forceSolidOn`, `axeOrPickaxe`, `horizontalBlockProvider(false)`, item tagged
`create:contraption_controlled`), `PloughBlock` (an `AttachedActorBlock`), `AllShapes.HARVESTER_BASE`,
`recipe/crafting/kinetics/mechanical_plough`, `AllBlockTags.PLOUGH_WHITELIST` and `PLOUGH_BLACKLIST`.
The Plough works on a moving structure (`native/api/src/machines/actor_tools.rs`).
"""

from gen_content import ANDESITE, GRAY, HORIZONTAL_FACING_STATE, block

# `AllShapes.HARVESTER_BASE`, as the Harvester's.
SHAPE = {"property": "facing", "values": {
    "north": [[0, 2 / 16, 13 / 16, 1, 14 / 16, 1]],
    "south": [[0, 2 / 16, 0, 1, 14 / 16, 3 / 16]],
    "east": [[0, 2 / 16, 0, 3 / 16, 14 / 16, 1]],
    "west": [[13 / 16, 2 / 16, 0, 1, 14 / 16, 1]],
}}

BLOCKS = {
    "mechanical_plough": block(HORIZONTAL_FACING_STATE, {"facing": "north"}, ANDESITE, GRAY, SHAPE,
                               render_shape="MODEL"),
}

TAB = ["mechanical_plough"]

NAMES = {"block.create.mechanical_plough": "Mechanical Plough"}

TAGS = {
    "create:item/contraption_controlled": ["create:mechanical_plough"],
    # `data/create/tags/block/plough_whitelist.json` and `plough_blacklist.json`.
    "create:block/plough_whitelist": ["minecraft:snow"],
    "create:block/plough_blacklist": ["#minecraft:portals"],
}

RECIPES = {
    # `recipe/crafting/kinetics/mechanical_plough`, in Horizon's 26.3 form.
    "crafting/kinetics/mechanical_plough": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["III", "AAA", " C "],
        "key": {"A": "create:andesite_alloy", "C": "create:andesite_casing", "I": "#c:plates/iron"},
        "result": {"id": "create:mechanical_plough", "count": 1},
    },
}
